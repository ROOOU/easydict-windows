import type { QueryEventPayload, QueryFinishedPayload, QueryResult } from "../types";

export interface QueryState {
  requestId: string | null;
  byIdentifier: Record<string, QueryResult>;
  lastIdentifier: string | null;
  running: boolean;
  finished: boolean;
}

export const emptyQueryState: QueryState = {
  requestId: null,
  byIdentifier: {},
  lastIdentifier: null,
  running: false,
  finished: false,
};

type QueryAction =
  | { type: "start"; requestId: string }
  | { type: "update"; payload: QueryEventPayload }
  | { type: "finish"; payload: QueryFinishedPayload }
  | { type: "stop" | "clear" }
  | { type: "reveal"; identifier: string };

function settleResults(results: Record<string, QueryResult>) {
  return Object.fromEntries(Object.entries(results).map(([identifier, result]) => [
    identifier, { ...result, isLoading: false, isStreamFinished: true },
  ]));
}

export function queryReducer(state: QueryState, action: QueryAction): QueryState {
  switch (action.type) {
    case "start":
      return { ...emptyQueryState, requestId: action.requestId, running: true };
    case "update": {
      const { requestId, identifier, result } = action.payload;
      if (requestId !== state.requestId) return state;
      const existing = state.byIdentifier[identifier];
      if (existing && result.generation < existing.generation) return state;
      return {
        ...state,
        byIdentifier: { ...state.byIdentifier, [identifier]: result },
        lastIdentifier: identifier,
      };
    }
    case "finish":
      if (action.payload.requestId !== state.requestId) return state;
      return {
        ...state,
        running: false,
        finished: !action.payload.cancelled,
        byIdentifier: action.payload.cancelled ? settleResults(state.byIdentifier) : state.byIdentifier,
      };
    case "stop":
      return { ...state, requestId: null, running: false, finished: false, byIdentifier: settleResults(state.byIdentifier) };
    case "clear":
      return emptyQueryState;
    case "reveal": {
      const result = state.byIdentifier[action.identifier];
      if (!result) return state;
      return {
        ...state,
        byIdentifier: { ...state.byIdentifier, [action.identifier]: { ...result, isShowing: true, manualShow: true } },
      };
    }
  }
}
