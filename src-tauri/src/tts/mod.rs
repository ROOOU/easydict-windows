//! Text-to-speech.
//!
//! Windows port of the macOS `TTSService` + `EZAudioPlayer` pair. Remote audio
//! still comes from the translation providers, but `AVPlayer` becomes
//! `Windows.Media.Playback.MediaPlayer`, and the offline system voice
//! (`NSSpeechSynthesizer`, upstream's `EZServiceTypeApple`) becomes
//! `Windows.Media.SpeechSynthesis.SpeechSynthesizer`.
//!
//! Windows-only: declare it as `#[cfg(windows)] mod tts;` in the crate root.
#![cfg(windows)]

use tokio::sync::{mpsc, oneshot};
use windows::core::{HSTRING, Result as WinResult};
use windows::Foundation::Uri;
use windows::Media::Core::MediaSource;
use windows::Media::Playback::MediaPlayer;
use windows::Media::SpeechSynthesis::{SpeechSynthesizer, VoiceInformation};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

use crate::config::Settings;
use crate::model::{Language, TtsServiceType};

/// `GoogleService`'s frozen `window.TKK` (see `service/google.rs`). That module
/// keeps both the constant and its `tts_url` private, so the value is mirrored
/// here instead of duplicating the whole signing routine.
const GOOGLE_TKK: &str = "444000.1270171236";
/// `translate_tts` refuses anything longer; upstream trims to the same limit.
const GOOGLE_TEXT_LIMIT: usize = 200;

#[derive(Debug, thiserror::Error)]
pub enum TtsError {
    #[error("no text to speak")]
    EmptyText,
    #[error("this service has no audio for the given text")]
    NoAudioUrl,
    #[error("platform error: {0}")]
    Platform(String),
}

/// Picks the TTS provider the way `TTSService` does: an English word prefers
/// Youdao when `prefer_youdao_tts_for_english_word` is set, otherwise the
/// configured `default_tts_service_type` decides.
///
/// `None` means "this provider has no remote audio for the text" — the caller
/// then speaks through [`TtsPlayer::speak`], which is what upstream falls back
/// to when a service fails to produce a URL.
///
/// The text being read is the query, so `from_language` is the language to
/// pronounce it in; `to_language` is used only when the source language is not
/// known (auto-detect or unsupported), where the target is the better guess.
pub fn tts_url(
    settings: &Settings,
    text: &str,
    from_language: Language,
    to_language: Language,
    is_english_word: bool,
) -> Option<String> {
    if text.trim().is_empty() {
        return None;
    }

    let language = spoken_language(from_language, to_language);
    // `EZAudioPlayer` only ever supplies an accent for English pronunciation.
    let accent = language
        .is_english()
        .then(|| settings.pronunciation_accent.as_str());

    // `EZAudioPlayer.playAudioURL:` — for an English word Youdao's recording is
    // preferred over the configured service, unless the user turned that off.
    if is_english_word && settings.prefer_youdao_tts_for_english_word {
        return Some(crate::service::youdao::tts_url(text, language, accent));
    }

    match settings.default_tts_service_type {
        TtsServiceType::Youdao => Some(crate::service::youdao::tts_url(text, language, accent)),
        TtsServiceType::Baidu => {
            let code = crate::service::baidu::tts_language_code(language, accent);
            Some(crate::service::baidu::tts_url(text, &code))
        }
        TtsServiceType::Google => google_tts_url(text, language),
        // `BingService.fetchTextToAudio` is a POST with an SSML body whose
        // `IG`/`IID` parameters are scraped from bing.com, so there is no
        // static URL to return here; playback falls back to the system
        // synthesiser. (Bing's own pronunciation audio still arrives through
        // `QueryResult` word phonetics.)
        TtsServiceType::Bing => None,
        // macOS `EZServiceTypeApple` — no URL at all, just `NSSpeechSynthesizer`.
        TtsServiceType::System => None,
    }
}

/// The language `text` should be pronounced in.
fn spoken_language(from: Language, to: Language) -> Language {
    match from {
        Language::Auto | Language::Unsupported => to,
        other => other,
    }
}

/// `GoogleService.textToAudio`: `translate_tts`, or nothing at all for English,
/// which Google only serves through its RPC path.
///
/// The URL template is the one `GoogleService::tts_url` builds; that method is
/// private to `service/google.rs`, so only its public `google_sign` half can be
/// reused from here.
fn google_tts_url(text: &str, language: Language) -> Option<String> {
    let code = crate::service::languages::default_language_code(
        crate::model::ServiceType::Google,
        language,
    )?;
    if code.starts_with("en") {
        return None;
    }

    let trimmed = crate::util::ns_trim_to_max_length(text, GOOGLE_TEXT_LIMIT);
    let encoded = crate::util::url_encode_query_allowed(&trimmed);
    let tk = crate::service::google::google_sign(&trimmed, GOOGLE_TKK);
    let len = crate::util::grapheme_count(&trimmed);
    Some(format!(
        "https://translate.google.com/translate_tts?ie=UTF-8&q={encoded}&tl={code}&total=1&idx=0&textlen={len}&tk={tk}&client=webapp&prev=input"
    ))
}

/// Plays audio and drives the system speech synthesiser.
///
/// The `MediaPlayer` and the `SpeechSynthesizer` are created per player and
/// owned by a dedicated `easydict-tts` thread, where every WinRT call happens;
/// the handle only posts commands to it. `TtsPlayer` therefore holds nothing
/// but a channel: it is `Send + Sync` without a single `unsafe impl`, and no
/// WinRT object has to honour cross-thread use.
pub struct TtsPlayer {
    worker: Worker,
}

impl TtsPlayer {
    pub fn new() -> Self {
        TtsPlayer {
            worker: Worker::start(),
        }
    }

    /// Streams a remote audio URL through `Windows.Media.Playback.MediaPlayer`
    /// (upstream's `AVPlayer`). Returns once playback has started; the audio
    /// keeps playing on the speech thread.
    pub async fn play_url(&self, url: &str) -> Result<(), TtsError> {
        if url.trim().is_empty() {
            return Err(TtsError::NoAudioUrl);
        }
        let (reply, answer) = oneshot::channel();
        self.worker.send(SpeechCommand::PlayUrl {
            url: url.to_string(),
            reply,
        })?;
        answer
            .await
            .map_err(|_| TtsError::Platform("the speech thread dropped the request".into()))?
    }

    /// Synthesises offline with `SpeechSynthesizer`, the Windows stand-in for
    /// `NSSpeechSynthesizer`. `language` selects the installed voice by its
    /// BCP-47 tag; when no voice matches, the synthesizer's default voice is
    /// used, as `AppleSpeechService` does when its lookup comes up empty.
    pub async fn speak(&self, text: &str, language: Language) -> Result<(), TtsError> {
        if text.trim().is_empty() {
            return Err(TtsError::EmptyText);
        }
        let (reply, answer) = oneshot::channel();
        self.worker.send(SpeechCommand::Speak {
            text: text.to_string(),
            locale: language.windows_locale().to_string(),
            reply,
        })?;
        answer
            .await
            .map_err(|_| TtsError::Platform("the speech thread dropped the request".into()))?
    }

    /// Stops whatever is playing or being spoken. Best effort: a dead worker
    /// has nothing left to stop.
    pub fn stop(&self) {
        let _ = self.worker.send(SpeechCommand::Stop);
    }
}

impl Default for TtsPlayer {
    fn default() -> Self {
        Self::new()
    }
}

/// A unit of work for the speech thread.
enum SpeechCommand {
    PlayUrl {
        url: String,
        reply: oneshot::Sender<Result<(), TtsError>>,
    },
    Speak {
        text: String,
        locale: String,
        reply: oneshot::Sender<Result<(), TtsError>>,
    },
    Stop,
}

/// Handle on the speech thread, plus the reason it could not be started.
enum Worker {
    Ready(mpsc::UnboundedSender<SpeechCommand>),
    Failed(String),
}

impl Worker {
    fn start() -> Worker {
        let (tx, mut rx) = mpsc::unbounded_channel::<SpeechCommand>();

        let spawned = std::thread::Builder::new()
            .name("easydict-tts".to_string())
            .spawn(move || {
                // WinRT activation needs an initialised apartment, and this
                // thread has no message pump, so it joins the MTA. `MediaPlayer`
                // and `SpeechSynthesizer` are both usable from there.
                let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };

                let mut state = SpeechState::default();
                while let Some(command) = rx.blocking_recv() {
                    match command {
                        SpeechCommand::PlayUrl { url, reply } => {
                            let _ = reply.send(state.play_url(&url));
                        }
                        SpeechCommand::Speak { text, locale, reply } => {
                            let _ = reply.send(state.speak(&text, &locale));
                        }
                        SpeechCommand::Stop => state.stop(),
                    }
                }
            });

        match spawned {
            Ok(_) => Worker::Ready(tx),
            Err(error) => Worker::Failed(format!("could not start the speech thread: {error}")),
        }
    }

    fn send(&self, command: SpeechCommand) -> Result<(), TtsError> {
        match self {
            Worker::Ready(tx) => tx
                .send(command)
                .map_err(|_| TtsError::Platform("the speech thread has stopped".into())),
            Worker::Failed(message) => Err(TtsError::Platform(message.clone())),
        }
    }
}

/// The WinRT objects the speech thread owns. They are created on first use so a
/// player that is never asked to play anything costs nothing.
#[derive(Default)]
struct SpeechState {
    player: Option<MediaPlayer>,
    synthesizer: Option<SpeechSynthesizer>,
}

impl SpeechState {
    fn player(&mut self) -> WinResult<&MediaPlayer> {
        if self.player.is_none() {
            self.player = Some(MediaPlayer::new()?);
        }
        Ok(self
            .player
            .as_ref()
            .expect("the player was just constructed"))
    }

    fn play_url(&mut self, url: &str) -> Result<(), TtsError> {
        let uri = Uri::CreateUri(&HSTRING::from(url)).map_err(platform)?;
        let source = MediaSource::CreateFromUri(&uri).map_err(platform)?;
        let player = self.player().map_err(platform)?;
        // Setting a source replaces whatever was playing before.
        player.SetSource(&source).map_err(platform)?;
        player.Play().map_err(platform)
    }

    fn speak(&mut self, text: &str, locale: &str) -> Result<(), TtsError> {
        if self.synthesizer.is_none() {
            self.synthesizer = Some(SpeechSynthesizer::new().map_err(platform)?);
        }
        let synthesizer = self
            .synthesizer
            .as_ref()
            .expect("the synthesizer was just constructed");

        if let Some(voice) = voice_for(locale) {
            synthesizer.SetVoice(&voice).map_err(platform)?;
        }

        // Synthesis runs offline and is still an async WinRT operation; this is
        // the speech thread, so blocking until it finishes is what we want.
        let stream = synthesizer
            .SynthesizeTextToStreamAsync(&HSTRING::from(text))
            .and_then(|operation| operation.join())
            .map_err(platform)?;
        let content_type = stream.ContentType().map_err(platform)?;
        // `SpeechSynthesisStream` is an `IRandomAccessStream`, so the same
        // `MediaPlayer` that plays remote URLs plays synthesised speech — which
        // is why one `stop` below silences both paths.
        let source = MediaSource::CreateFromStream(&stream, &content_type).map_err(platform)?;

        let player = self.player().map_err(platform)?;
        player.SetSource(&source).map_err(platform)?;
        player.Play().map_err(platform)
    }

    fn stop(&mut self) {
        if let Some(player) = &self.player {
            // Pausing is enough: synthesis does not own a voice of its own once
            // it has been turned into a media source. (`NSSpeechSynthesizer`
            // needed its own `stopSpeaking`, which has no counterpart here.)
            let _ = player.Pause();
        }
    }
}

/// `AppleSpeechService.voiceIdentifier(for:)` — the installed voice for a
/// language tag, falling back to the same primary language before letting the
/// synthesizer keep its default voice.
fn voice_for(locale: &str) -> Option<VoiceInformation> {
    let voices = SpeechSynthesizer::AllVoices().ok()?;
    let count = voices.Size().ok()?;
    let primary = locale.split('-').next().unwrap_or(locale);

    let mut same_language = None;
    for index in 0..count {
        let Ok(voice) = voices.GetAt(index) else {
            continue;
        };
        // `VoiceInformation::Language` is already the BCP-47 tag.
        let Ok(tag) = voice.Language() else {
            continue;
        };
        if tag == locale {
            return Some(voice);
        }
        if same_language.is_none() && tag.to_string_lossy().starts_with(primary) {
            same_language = Some(voice);
        }
    }
    same_language
}

fn platform(error: windows::core::Error) -> TtsError {
    TtsError::Platform(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(service: TtsServiceType, prefer_youdao: bool, accent: &str) -> Settings {
        let mut settings = Settings::default();
        settings.default_tts_service_type = service;
        settings.prefer_youdao_tts_for_english_word = prefer_youdao;
        settings.pronunciation_accent = accent.to_string();
        settings
    }

    #[test]
    fn an_english_word_prefers_youdao() {
        // Google is configured, but an English word goes to Youdao anyway.
        let config = settings(TtsServiceType::Google, true, "us");
        let url = tts_url(
            &config,
            "good",
            Language::English,
            Language::SimplifiedChinese,
            true,
        )
        .unwrap();
        assert_eq!(
            url,
            "https://dict.youdao.com/dictvoice?audio=good&le=en&type=2"
        );
    }

    #[test]
    fn the_english_word_preference_can_be_turned_off() {
        // Google genuinely has no `translate_tts` voice for English, so the
        // preference being off really does change the outcome to "no audio".
        let config = settings(TtsServiceType::Google, false, "us");
        assert!(
            tts_url(
                &config,
                "good",
                Language::English,
                Language::SimplifiedChinese,
                true
            )
            .is_none()
        );
        // With the preference on, Youdao answers for the same input.
        let config = settings(TtsServiceType::Google, true, "us");
        assert!(
            tts_url(
                &config,
                "good",
                Language::English,
                Language::SimplifiedChinese,
                true
            )
            .is_some()
        );
    }

    #[test]
    fn the_accent_only_reaches_english() {
        let settings = settings(TtsServiceType::Youdao, false, "uk");
        let english = tts_url(
            &settings,
            "good",
            Language::English,
            Language::SimplifiedChinese,
            false,
        )
        .unwrap();
        assert!(english.ends_with("&le=en&type=1"));

        let chinese = tts_url(
            &settings,
            "你好",
            Language::SimplifiedChinese,
            Language::English,
            false,
        )
        .unwrap();
        assert!(chinese.contains("&le=zh&type=2"));
    }

    #[test]
    fn the_default_configuration_speaks_with_youdao() {
        let settings = Settings::default();
        assert_eq!(settings.default_tts_service_type, TtsServiceType::Youdao);
        assert!(settings.prefer_youdao_tts_for_english_word);

        let url = tts_url(
            &settings,
            "你好",
            Language::SimplifiedChinese,
            Language::English,
            false,
        )
        .unwrap();
        assert!(url.starts_with("https://dict.youdao.com/dictvoice?audio="));
        assert!(url.contains("&le=zh&"));
        // Chinese is the common case here, and the word being spoken is the
        // whole payload — it has to arrive as UTF-8 percent-escapes.
        assert!(url.contains("audio=%E4%BD%A0%E5%A5%BD&"), "{url}");
    }

    #[test]
    fn baidu_uses_its_own_language_code_table() {
        let settings = settings(TtsServiceType::Baidu, true, "uk");
        let url = tts_url(
            &settings,
            "good",
            Language::English,
            Language::SimplifiedChinese,
            false,
        )
        .unwrap();
        assert_eq!(
            url,
            "https://fanyi.baidu.com/gettts?text=good&lan=uk&spd=3&source=web"
        );
    }

    #[test]
    fn google_has_no_english_audio_but_has_chinese_audio() {
        let settings = settings(TtsServiceType::Google, false, "us");
        assert!(
            tts_url(
                &settings,
                "hello",
                Language::English,
                Language::SimplifiedChinese,
                false
            )
            .is_none()
        );

        let url = tts_url(
            &settings,
            "你好",
            Language::SimplifiedChinese,
            Language::English,
            false,
        )
        .unwrap();
        assert!(url.starts_with("https://translate.google.com/translate_tts?ie=UTF-8&q="));
        assert!(url.contains("&tl=zh-CN&total=1&idx=0&textlen=2&tk="));
        assert!(url.contains("&q=%E4%BD%A0%E5%A5%BD&"), "{url}");
    }

    #[test]
    fn the_system_voice_and_bing_have_no_url() {
        let system = settings(TtsServiceType::System, false, "us");
        assert!(
            tts_url(
                &system,
                "good",
                Language::English,
                Language::SimplifiedChinese,
                false
            )
            .is_none()
        );

        let bing = settings(TtsServiceType::Bing, false, "us");
        assert!(
            tts_url(
                &bing,
                "你好",
                Language::SimplifiedChinese,
                Language::English,
                false
            )
            .is_none()
        );
    }

    #[test]
    fn empty_text_is_never_spoken() {
        let settings = Settings::default();
        assert!(
            tts_url(
                &settings,
                "   ",
                Language::English,
                Language::SimplifiedChinese,
                true
            )
            .is_none()
        );
    }

    #[test]
    fn an_unknown_source_language_falls_back_to_the_target() {
        let settings = settings(TtsServiceType::Baidu, false, "us");
        let url = tts_url(
            &settings,
            "hello",
            Language::Auto,
            Language::English,
            false,
        )
        .unwrap();
        assert!(url.contains("lan=en"));
    }

    #[test]
    fn the_player_can_cross_threads() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<TtsPlayer>();
    }
}
