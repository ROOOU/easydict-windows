import type { OcrOutput, QueryResult } from "../types";

export interface OcrState {
  output: OcrOutput | null;
  text: string;
  translation: string | null;
  translationService: string | null;
  recognitionId: string | null;
  translationId: string | null;
  error: string | null;
}

export const emptyOcrState: OcrState = {
  output: null,
  text: "",
  translation: null,
  translationService: null,
  recognitionId: null,
  translationId: null,
  error: null,
};

type OcrAction =
  | { type: "capture"; id: string }
  | { type: "recognized"; id: string; output: OcrOutput }
  | { type: "captureError"; id: string; error: string }
  | { type: "captureEnd"; id: string }
  | { type: "edit"; text: string }
  | { type: "translate"; id: string }
  | { type: "translated"; id: string; result: QueryResult }
  | { type: "translationError"; id: string; error: string }
  | { type: "cancel" }
  | { type: "error"; error: string | null };

export function ocrReducer(state: OcrState, action: OcrAction): OcrState {
  switch (action.type) {
    case "capture":
      return { ...state, recognitionId: action.id, translationId: null, error: null };
    case "recognized":
      if (state.recognitionId !== action.id) return state;
      return {
        ...state, output: action.output, text: action.output.text,
        translation: null, translationService: null, translationId: null,
      };
    case "captureError":
      return state.recognitionId === action.id ? { ...state, error: action.error } : state;
    case "captureEnd":
      return state.recognitionId === action.id ? { ...state, recognitionId: null } : state;
    case "edit":
      return { ...emptyOcrState, text: action.text };
    case "translate":
      return { ...state, translationId: action.id, error: null };
    case "translated": {
      if (state.translationId !== action.id) return state;
      const translation = (action.result.translatedResults ?? []).join("\n");
      return {
        ...state, translationId: null,
        translation: translation || null,
        translationService: translation ? action.result.serviceName : null,
        error: translation ? null : (action.result.error?.message ?? "没有返回翻译结果。"),
      };
    }
    case "translationError":
      return state.translationId === action.id
        ? { ...state, translationId: null, translation: null, translationService: null, error: action.error }
        : state;
    case "cancel":
      return { ...state, recognitionId: null, translationId: null };
    case "error":
      return { ...state, error: action.error };
  }
}
