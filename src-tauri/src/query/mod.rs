//! Query orchestration.
//!
//! Mirrors the macOS `EZQueryService` lifecycle:
//!
//! * every service keeps a **result generation** counter and a current result;
//! * starting a query bumps the generation and resets the result;
//! * a stop block is registered on the query model, keyed by service id, so a
//!   new query can cancel every in-flight request;
//! * a stream that yields after its generation went stale is discarded.
//!
//! The engine fans the query out to every enabled service concurrently and
//! forwards `QueryEvent`s to whatever is driving the UI.

use std::sync::Arc;

use parking_lot::Mutex;
use tokio::sync::{mpsc::UnboundedSender, watch};

use crate::model::{ActionType, Language, QueryError, QueryModel, QueryResult, ResultState, ServiceType};
use crate::service::{Prehandle, QueryService, ServiceUpdate};

/// A cancellation signal that can be cloned and polled.
#[derive(Clone)]
pub struct Cancellation {
    rx: watch::Receiver<bool>,
}

impl Cancellation {
    pub fn is_cancelled(&self) -> bool {
        *self.rx.borrow()
    }

    /// Resolves once the owning run is cancelled.
    pub async fn cancelled(&mut self) {
        if *self.rx.borrow() {
            return;
        }
        // `changed()` only resolves on a *new* value, so loop until true.
        while self.rx.changed().await.is_ok() {
            if *self.rx.borrow() {
                return;
            }
        }
    }
}

/// The write half of a [`Cancellation`].
///
/// Cloning it shares the same channel, so a request's own stop block and the
/// run-wide one can both cancel the same wait.
#[derive(Clone)]
pub struct CancellationSource {
    tx: watch::Sender<bool>,
}

impl CancellationSource {
    pub fn new() -> Self {
        let (tx, _rx) = watch::channel(false);
        CancellationSource { tx }
    }

    pub fn cancellation(&self) -> Cancellation {
        Cancellation {
            rx: self.tx.subscribe(),
        }
    }

    pub fn cancel(&self) {
        self.tx.send_replace(true);
    }
}

impl Default for CancellationSource {
    fn default() -> Self {
        Self::new()
    }
}

/// Per-service mutable state, replacing the ObjC properties on `EZQueryService`.
pub struct ServiceState {
    /// Bumped whenever a new query starts, invalidating older streams.
    generation: Mutex<u64>,
    result: Mutex<Option<QueryResult>>,
    /// `queryModel.setStop(_:serviceType:)`
    stop: Mutex<Option<CancellationSource>>,
}

impl Default for ServiceState {
    fn default() -> Self {
        ServiceState {
            generation: Mutex::new(0),
            result: Mutex::new(None),
            stop: Mutex::new(None),
        }
    }
}

impl ServiceState {
    pub fn generation(&self) -> u64 {
        *self.generation.lock()
    }

    pub fn result(&self) -> Option<QueryResult> {
        self.result.lock().clone()
    }

    pub fn set_result(&self, result: QueryResult) {
        *self.result.lock() = Some(result);
    }

    pub fn update_result(&self, f: impl FnOnce(&mut QueryResult)) {
        let mut guard = self.result.lock();
        if let Some(r) = guard.as_mut() {
            f(r);
        }
    }

    /// `stopAllService()` — cancels and clears the registered stop block.
    pub fn stop(&self) {
        if let Some(source) = self.stop.lock().take() {
            source.cancel();
        }
    }
}

/// A service plus its query state.
pub struct ServiceHandle {
    pub service: Arc<dyn QueryService>,
    pub state: Arc<ServiceState>,
}

impl ServiceHandle {
    pub fn new(service: Arc<dyn QueryService>) -> Self {
        ServiceHandle {
            service,
            state: Arc::new(ServiceState::default()),
        }
    }

    pub fn identifier(&self) -> &str {
        self.service.identifier()
    }

    pub fn service_type(&self) -> ServiceType {
        self.service.service_type()
    }

    /// `createNewResult()` — bumps the generation, then hands back a fresh
    /// result the caller may mutate before it becomes visible.
    pub fn begin_new_result(&self, query: &QueryModel) -> (u64, QueryResult) {
        let generation = {
            let mut g = self.state.generation.lock();
            *g = g.wrapping_add(1);
            *g
        };
        let mut result = self.service.new_result(query);
        result.generation = generation;
        result.show_replace_button = matches!(
            query.action_type,
            ActionType::AutoSelectQuery | ActionType::ShortcutQuery | ActionType::InvokeQuery
        );
        *self.state.result.lock() = Some(result.clone());
        (generation, result)
    }
}

/// Events emitted while a query runs.
#[derive(Debug, Clone)]
pub enum QueryEvent {
    /// A service produced or updated its result.
    Update {
        identifier: String,
        service_type: ServiceType,
        result: Box<QueryResult>,
    },
    /// Every service has settled.
    AllFinished { cancelled: bool },
}

/// Runs one fan-out query.
pub struct QueryRun {
    cancellation: CancellationSource,
    handles: Vec<Arc<ServiceHandle>>,
}

impl QueryRun {
    /// `queryAllSerives(_:)` — dispatches to every handle, in order.
    ///
    /// Returns immediately; results arrive on `tx`.
    pub fn start(
        handles: Vec<Arc<ServiceHandle>>,
        model: QueryModel,
        tx: UnboundedSender<QueryEvent>,
    ) -> QueryRun {
        let cancellation = CancellationSource::new();
        let token = cancellation.cancellation();
        let handles: Vec<_> = handles
            .iter()
            .map(|handle| Arc::new(ServiceHandle::new(handle.service.clone())))
            .collect();
        let run_handles = handles.clone();

        tokio::spawn(async move {
            let mut tasks = Vec::with_capacity(run_handles.len());

            for handle in run_handles {
                let model = model.clone();
                let tx = tx.clone();
                let token = token.clone();
                tasks.push(tokio::spawn(async move {
                    run_one(handle, model, tx, token).await;
                }));
            }

            for task in tasks {
                let _ = task.await;
            }
            let _ = tx.send(QueryEvent::AllFinished {
                cancelled: token.is_cancelled(),
            });
        });

        QueryRun { cancellation, handles }
    }

    /// `stopAllService()`
    pub fn stop(&self) {
        self.cancellation.cancel();
    }

    pub fn cancellation(&self) -> Cancellation {
        self.cancellation.cancellation()
    }

    pub fn show_service_result(&self, identifier: &str) {
        if let Some(handle) = self.handles.iter().find(|handle| handle.identifier() == identifier) {
            handle.state.update_result(|result| {
                result.manual_show = true;
                result.is_showing = true;
            });
        }
    }
}

/// `queryWithModel:service:completion:`
async fn run_one(
    handle: Arc<ServiceHandle>,
    model: QueryModel,
    tx: UnboundedSender<QueryEvent>,
    mut token: Cancellation,
) {
    let service = &handle.service;
    if model.query_text.is_empty() || token.is_cancelled() {
        return;
    }

    let (generation, mut result) = handle.begin_new_result(&model);
    result.is_showing = true;
    result.is_loading = true;
    let _ = tx.send(QueryEvent::Update {
        identifier: service.identifier().to_string(),
        service_type: service.service_type(),
        result: Box::new(result.clone()),
    });

    // A fresh cancellation source per request, superseding any older one.
    let source = CancellationSource::new();
    *handle.state.stop.lock() = Some(source.clone());
    let mut inner = source.cancellation();

    let (update_tx, mut update_rx) = tokio::sync::mpsc::unbounded_channel::<ServiceUpdate>();

    let outcome = {
        let translate = service.translate(&model, Some(update_tx.clone()));
        tokio::pin!(translate);

        let mut final_result: Option<Result<QueryResult, QueryError>> = None;
        loop {
            tokio::select! {
                biased;
                _ = token.cancelled() => break,
                _ = inner.cancelled() => break,
                Some(update) = update_rx.recv() => {
                    if handle.state.generation() != generation {
                        break;
                    }
                    match update {
                        ServiceUpdate::Chunk(chunk) => {
                            handle.state.update_result(|r| {
                                r.append_stream_chunk(&chunk);
                            });
                            if let Some(r) = handle.state.result() {
                                let _ = tx.send(QueryEvent::Update {
                                    identifier: service.identifier().to_string(),
                                    service_type: service.service_type(),
                                    result: Box::new(r),
                                });
                            }
                        }
                        ServiceUpdate::Validation(note) => {
                            handle.state.update_result(|r| r.validation_message = Some(note));
                            if let Some(r) = handle.state.result() {
                                let _ = tx.send(QueryEvent::Update {
                                    identifier: service.identifier().to_string(),
                                    service_type: service.service_type(),
                                    result: Box::new(r),
                                });
                            }
                        }
                    }
                }
                res = &mut translate => {
                    final_result = Some(res);
                    break;
                }
            }
        }

        // Drain anything the service already queued before finishing.
        while let Ok(update) = update_rx.try_recv() {
            if let ServiceUpdate::Chunk(chunk) = update {
                handle.state.update_result(|r| r.append_stream_chunk(&chunk));
            } else if let ServiceUpdate::Validation(note) = update {
                handle.state.update_result(|r| r.validation_message = Some(note));
            }
        }

        final_result
    };

    if handle.state.generation() != generation {
        // A newer query took over; drop this result silently.
        return;
    }

    // Fold the outcome into the stored result, preserving streamed text.
    match outcome {
        Some(Ok(mut r)) => {
            r.generation = generation;
            // The service builds a fresh result, which starts hidden. Carrying
            // the flags of the loading result over is what keeps the card the
            // user is already looking at from disappearing when the text
            // arrives.
            if let Some(existing) = handle.state.result() {
                if r.translated_results.is_none() {
                    r.translated_results = existing.translated_results;
                    if r.word_result.is_none() {
                        r.word_result = existing.word_result;
                    }
                }
                r.manual_show |= existing.manual_show;
                r.is_showing = existing.is_showing;
            }
            if r.has_translated_result() {
                // A finished translation is always worth showing.
                r.is_showing = true;
            }
            if service.auto_convert_traditional_chinese()
                && model.query_target_language == Language::TraditionalChinese
            {
                crate::model::convert_result_to_traditional(&mut r);
            }
            r.state = ResultState::Finished;
            r.is_loading = false;
            r.is_stream_finished = true;
            let hide = !r.manual_show && !r.has_translated_result() && r.is_warning_error();
            if hide {
                r.is_showing = false;
            }
            handle.state.set_result(r.clone());
            let _ = tx.send(QueryEvent::Update {
                identifier: service.identifier().to_string(),
                service_type: service.service_type(),
                result: Box::new(r),
            });
        }
        Some(Err(err)) => {
            handle.state.update_result(|r| {
                r.error = Some(err.clone());
                r.state = ResultState::Finished;
                r.is_loading = false;
                r.is_stream_finished = true;
                if !r.manual_show && !r.has_translated_result() && err.kind.is_warning() {
                    r.is_showing = false;
                }
            });
            if let Some(r) = handle.state.result() {
                let _ = tx.send(QueryEvent::Update {
                    identifier: service.identifier().to_string(),
                    service_type: service.service_type(),
                    result: Box::new(r),
                });
            }
        }
        None => {
            // Cancelled.
            handle.state.update_result(|r| {
                r.is_loading = false;
                r.is_stream_finished = true;
            });
            if let Some(result) = handle.state.result() {
                let _ = tx.send(QueryEvent::Update {
                    identifier: service.identifier().to_string(),
                    service_type: service.service_type(),
                    result: Box::new(result),
                });
            }
        }
    }
}

/// Runs a service without going through the event channel — used by the
/// aggregate commands (`translate_aggregate`, OCR, TTS) and by tests.
pub async fn run_direct(
    handle: &ServiceHandle,
    model: &QueryModel,
) -> Result<QueryResult, QueryError> {
    let (_generation, _) = handle.begin_new_result(model);

    match handle.service.prehandle(model)? {
        Prehandle::Handled(result) => {
            handle.state.set_result(result.clone());
            Ok(result)
        }
        Prehandle::Proceed => {
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ServiceUpdate>();
            let mut result = handle.service.translate(model, Some(tx.clone())).await?;

            // A streaming service assembles its finished text from the very
            // deltas it forwards, so anything still queued is already reflected
            // in `result` — folding it in again doubles the translation. The
            // queued text is kept only as a fallback for a service that streams
            // without returning the assembly.
            let mut streamed = String::new();
            let mut note = None;
            while let Ok(update) = rx.try_recv() {
                match update {
                    ServiceUpdate::Chunk(chunk) => streamed.push_str(&chunk),
                    ServiceUpdate::Validation(message) => note = Some(message),
                }
            }
            if result.translated_results.is_none() && !streamed.is_empty() {
                result.translated_results = Some(vec![streamed]);
            }
            if result.validation_message.is_none() {
                result.validation_message = note;
            }
            if handle
                .service
                .auto_convert_traditional_chinese()
                && model.query_target_language == Language::TraditionalChinese
            {
                crate::model::convert_result_to_traditional(&mut result);
            }
            result.state = ResultState::Finished;
            result.is_loading = false;
            result.is_stream_finished = true;
            handle.state.set_result(result.clone());
            Ok(result)
        }
    }
}

/// The stop-block registry equivalent: `EZQueryModel.stopAllService()`.
pub fn stop_all(handles: &[Arc<ServiceHandle>]) {
    for handle in handles {
        handle.state.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Settings;
    use crate::service::{ServiceContext, UpdateSender};
    use async_trait::async_trait;

    struct Slow {
        ctx: ServiceContext,
    }

    #[async_trait]
    impl QueryService for Slow {
        fn service_type(&self) -> ServiceType {
            ServiceType::Google
        }
        fn identifier(&self) -> &str {
            "Google"
        }
        fn ctx(&self) -> &ServiceContext {
            &self.ctx
        }
        async fn translate(
            &self,
            query: &QueryModel,
            updates: Option<UpdateSender>,
        ) -> Result<QueryResult, QueryError> {
            let mut r = self.new_result(query);
            for i in 0..5 {
                if let Some(tx) = &updates {
                    let _ = tx.send(ServiceUpdate::Chunk(format!("{i}")));
                }
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
            r.state = ResultState::Finished;
            Ok(r)
        }
    }

    fn handle() -> ServiceHandle {
        let settings = Arc::new(Settings::default());
        let ctx = ServiceContext::new(settings, ServiceType::Google, "Google");
        ServiceHandle::new(Arc::new(Slow { ctx }))
    }

    /// A service that answers with a warning and no text, which the UI hides.
    struct Empty {
        ctx: ServiceContext,
    }

    #[async_trait]
    impl QueryService for Empty {
        fn service_type(&self) -> ServiceType {
            ServiceType::Bing
        }
        fn identifier(&self) -> &str {
            "Bing"
        }
        fn ctx(&self) -> &ServiceContext {
            &self.ctx
        }
        async fn translate(
            &self,
            _query: &QueryModel,
            _updates: Option<UpdateSender>,
        ) -> Result<QueryResult, QueryError> {
            Err(QueryError::new(crate::model::ErrorType::NoResult))
        }
    }

    fn empty_handle() -> ServiceHandle {
        let settings = Arc::new(Settings::default());
        let ctx = ServiceContext::new(settings, ServiceType::Bing, "Bing");
        ServiceHandle::new(Arc::new(Empty { ctx }))
    }

    #[tokio::test]
    async fn a_finished_translation_stays_visible() {
        // The finished result is a fresh object built by the service; it starts
        // hidden and must not replace the loading card with an invisible one.
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let _run = QueryRun::start(vec![Arc::new(handle())], QueryModel::new("hello"), tx);
        let (results, _) = finished_events(rx).await;
        let last = results.last().expect("a final update");
        assert!(last.is_showing, "a card with translated text must stay visible");
        assert_eq!(last.translated_text().as_deref(), Some("01234"));
    }

    #[tokio::test]
    async fn a_warning_without_text_is_hidden() {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let _run = QueryRun::start(vec![Arc::new(empty_handle())], QueryModel::new("hello"), tx);
        let (results, _) = finished_events(rx).await;
        let last = results.last().expect("a final update");
        assert!(!last.is_showing);
        assert!(last.error.is_some());
    }

    #[test]
    fn generation_advances_per_query() {
        let h = handle();
        let model = QueryModel::new("hello");
        let (g1, _) = h.begin_new_result(&model);
        let (g2, _) = h.begin_new_result(&model);
        assert!(g2 > g1);
    }

    #[test]
    fn cancellation_is_remembered_before_a_receiver_subscribes() {
        let source = CancellationSource::new();
        source.cancel();
        assert!(source.cancellation().is_cancelled());
    }

    async fn finished_events(
        mut receiver: tokio::sync::mpsc::UnboundedReceiver<QueryEvent>,
    ) -> (Vec<QueryResult>, bool) {
        tokio::time::timeout(std::time::Duration::from_secs(2), async move {
            let mut results = Vec::new();
            while let Some(event) = receiver.recv().await {
                match event {
                    QueryEvent::Update { result, .. } => results.push(*result),
                    QueryEvent::AllFinished { cancelled } => return (results, cancelled),
                }
            }
            panic!("query ended without a completion event");
        }).await.expect("query did not settle")
    }

    #[tokio::test]
    async fn parallel_windows_keep_their_results_separate() {
        let shared = Arc::new(handle());
        let (first_tx, first_rx) = tokio::sync::mpsc::unbounded_channel();
        let (second_tx, second_rx) = tokio::sync::mpsc::unbounded_channel();
        let _first = QueryRun::start(vec![shared.clone()], QueryModel::new("first"), first_tx);
        let _second = QueryRun::start(vec![shared.clone()], QueryModel::new("second"), second_tx);
        let (first, second) = tokio::join!(finished_events(first_rx), finished_events(second_rx));
        for (events, cancelled, text) in [(first.0, first.1, "first"), (second.0, second.1, "second")] {
            assert!(!cancelled);
            assert!(events.iter().all(|result| result.query_text == text));
            let last = events.last().unwrap();
            assert!(!last.is_loading);
            assert_eq!(last.translated_text().as_deref(), Some("01234"));
        }
        assert!(shared.state.result().is_none());
    }

    #[tokio::test]
    async fn cancelling_one_window_does_not_cancel_another() {
        let shared = Arc::new(handle());
        let (first_tx, mut first_rx) = tokio::sync::mpsc::unbounded_channel();
        let (second_tx, second_rx) = tokio::sync::mpsc::unbounded_channel();
        let first = QueryRun::start(vec![shared.clone()], QueryModel::new("cancel"), first_tx);
        let _second = QueryRun::start(vec![shared], QueryModel::new("keep"), second_tx);
        assert!(matches!(first_rx.recv().await, Some(QueryEvent::Update { .. })));
        first.stop();
        let (cancelled, completed) = tokio::join!(finished_events(first_rx), finished_events(second_rx));
        assert!(cancelled.1);
        assert!(!cancelled.0.last().unwrap().is_loading);
        assert!(!completed.1);
        assert_eq!(completed.0.last().unwrap().translated_text().as_deref(), Some("01234"));
    }

    #[tokio::test]
    async fn a_run_cancelled_before_dispatch_does_not_start_a_service() {
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        let run = QueryRun::start(vec![Arc::new(handle())], QueryModel::new("cancel"), sender);
        run.stop();
        let (results, cancelled) = finished_events(receiver).await;
        assert!(cancelled);
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn chunks_are_the_fallback_when_the_service_returns_no_text() {
        // `Slow` forwards deltas but never assembles them into its result, so
        // the drained chunks are the only text there is.
        let h = handle();
        let result = run_direct(&h, &QueryModel::new("hello")).await.unwrap();
        assert_eq!(result.translated_text().as_deref(), Some("01234"));
    }

    #[tokio::test]
    async fn cancellation_stops_the_run() {
        let h = Arc::new(handle());
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let run = QueryRun::start(vec![h], QueryModel::new("hello"), tx);
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        run.stop();
        // The first update always goes out before the request body runs.
        let first = rx.recv().await;
        assert!(first.is_some());
    }

    /// A streaming service shaped the way the real ones are: it pushes each
    /// delta *and* returns a result that already carries the full text, because
    /// `StreamService.translate` assembles the finished string from the same
    /// deltas it forwarded. [`Slow`] does neither, which is why it cannot catch
    /// the two being folded together twice.
    struct Streaming {
        ctx: ServiceContext,
    }

    #[async_trait]
    impl QueryService for Streaming {
        fn service_type(&self) -> ServiceType {
            ServiceType::DeepSeek
        }
        fn identifier(&self) -> &str {
            "DeepSeek"
        }
        fn ctx(&self) -> &ServiceContext {
            &self.ctx
        }
        async fn translate(
            &self,
            query: &QueryModel,
            updates: Option<UpdateSender>,
        ) -> Result<QueryResult, QueryError> {
            let mut r = self.new_result(query);
            for piece in ["你好", "，世界"] {
                if let Some(tx) = &updates {
                    let _ = tx.send(ServiceUpdate::Chunk(piece.to_string()));
                }
            }
            r.translated_results = Some(vec!["你好，世界".to_string()]);
            r.state = ResultState::Finished;
            Ok(r)
        }
    }

    #[tokio::test]
    async fn run_direct_does_not_double_the_streamed_text() {
        // A key is needed to get past `prehandle`'s gate; the request never
        // leaves this process.
        let mut settings = Settings::default();
        settings
            .services
            .entry(ServiceType::DeepSeek.raw().to_string())
            .or_default()
            .api_key = "key".to_string();
        let ctx = ServiceContext::new(
            Arc::new(settings),
            ServiceType::DeepSeek,
            "DeepSeek",
        );
        let h = ServiceHandle::new(Arc::new(Streaming { ctx }));

        let result = run_direct(&h, &QueryModel::new("hello")).await.unwrap();

        assert_eq!(
            result.translated_text().as_deref(),
            Some("你好，世界"),
            "the deltas and the finished text describe the same translation"
        );
    }
}
