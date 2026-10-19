/**
 * TypeScript mirror of the Rust command surface.
 *
 * Every type here is the JSON `serde` produces from the matching Rust struct.
 * The Rust side uses `#[serde(rename_all = "camelCase")]` throughout, so the
 * field names below match `commands.rs`, `config/mod.rs`, `model/mod.rs` and
 * `model/result.rs` one-for-one. When a Rust field is added, add it here too —
 * `tsc` is the only thing that will notice.
 */

// ---------------------------------------------------------------------------
// Language
// ---------------------------------------------------------------------------

/**
 * A language identifier. Serialises to the raw upstream string — `"auto"`,
 * `"Simplified-Chinese"`, `"English"`, … — so settings files stay compatible
 * with the macOS app. Never display this directly; look the entry up in
 * `supportedLanguages()` and use `chineseName` / `englishName`.
 */
export type Language = string;

export interface LanguageInfo {
  value: Language;
  code: string;
  englishName: string;
  chineseName: string;
  nativeName: string;
  flag: string;
}

// ---------------------------------------------------------------------------
// Query results
// ---------------------------------------------------------------------------

export type ErrorType =
  | "unknown"
  | "api"
  | "parameter"
  | "unsupportedLanguage"
  | "missingSecretKey"
  | "noResult"
  | "timeout"
  | "unsupportedQueryType"
  | "unsupportedServiceType"
  | "contentTypeMismatch";

export interface QueryError {
  type: ErrorType;
  message: string | null;
  errorDataMessage: string | null;
  promptTitle: string | null;
  promptUrl: string | null;
}

export type ResultState = "idle" | "loading" | "streaming" | "finished" | "failed";

export interface WordPhonetic {
  word: string;
  language: Language;
  value: string | null;
  speakUrl: string | null;
  /** Localised label: 美式发音 / 英式发音 / 中文发音. */
  name: string | null;
  /** "us" | "uk" */
  accent: string | null;
}

export interface TranslatePart {
  means: string[];
  /** Abbreviated part of speech, e.g. `adj.` */
  part: string | null;
}

export interface TranslateExchange {
  name: string;
  words: string[];
}

export interface TranslateSimpleWord {
  word: string;
  part: string | null;
  means: string[] | null;
  /** `means.joined("; ")` */
  meansText: string;
  /** When true, prefix `meansText` with the part of speech. */
  showPartMeans: boolean;
}

export interface TranslateWordResult {
  phonetics: WordPhonetic[] | null;
  parts: TranslatePart[] | null;
  exchanges: TranslateExchange[] | null;
  simpleWords: TranslateSimpleWord[] | null;
  tags: string[] | null;
  etymology: string | null;
  synonyms: TranslatePart[] | null;
  antonyms: TranslatePart[] | null;
  collocation: TranslatePart[] | null;
}

export interface QueryResult {
  /** `"OpenAI"` or `"CustomOpenAI#<uuid>"`. */
  serviceTypeWithUniqueIdentifier: string;
  serviceName: string;

  queryText: string;
  from: Language;
  to: Language;

  state: ResultState;

  translatedResults: string[] | null;
  wordResult: TranslateWordResult | null;
  /** Rendered HTML for local dictionaries (MDict). */
  htmlString: string | null;

  error: QueryError | null;
  validationMessage: string | null;

  fromSpeakUrl: string | null;
  toSpeakUrl: string | null;

  isStream: boolean;
  markdownRenderingOverride: boolean | null;
  markdownEnabled: boolean;
  showReplaceButton: boolean;

  link: string | null;

  isShowing: boolean;
  isLoading: boolean;
  isStreamFinished: boolean;
  manualShow: boolean;
  generation: number;
}

export type ServiceType = string;

export interface QueryEventPayload {
  requestId: string;
  identifier: string;
  serviceType: ServiceType;
  result: QueryResult;
}

export interface QueryFinishedPayload {
  requestId: string;
  cancelled: boolean;
}

export interface ActionPayload {
  action: string;
  text?: string | null;
  selectionTarget?: [number, number] | null;
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

export type ServiceUsageStatus = "0" | "1" | "2";
export type ReasoningEffort = "off" | "high" | "max";
export type ApiKeyRequirement = "none" | "builtIn" | "userProvided" | "agentCli";
export type ServiceApiType = "web" | "secretKey";
export type DeepLAPIUsagePriority = "webFirst" | "authKeyFirst" | "authKeyOnly";
export type AppearanceType = "light" | "dark" | "followSystem";
export type ShowWindowPosition = "right" | "mouse" | "former" | "center";
export type WindowType = "main" | "mini" | "fixed";
export type LanguageDetectOptimize = "none" | "baidu" | "google";
export type TtsServiceType = "Youdao" | "Bing" | "Google" | "Baidu" | "System";

export interface ServiceConfig {
  usageStatus: ServiceUsageStatus;
  translation: boolean;
  dictionary: boolean;
  sentence: boolean;
  intelligentQueryTextType: number;

  supportedModels: string[];
  validModels: string[];
  model: string;
  apiKey: string;
  endpoint: string;
  name: string;

  enableCustomPrompt: boolean;
  systemPrompt: string;
  userPrompt: string;

  hideThinkTag: boolean;
  temperature: number;
  enableStreaming: boolean;
  reasoningEffort: ReasoningEffort;
  cliEffort: string;

  enabled: boolean;
}

export interface QueryRecord {
  queryText: string;
  fromLanguage: Language;
  toLanguage: Language;
  /** Unix milliseconds. */
  timestamp: number;
  translatedText?: string | null;
}

/**
 * The whole persisted settings document. Every field is required on the way
 * out; on the way in, `updateSettings` accepts a deep partial and merges it.
 */
export interface DesktopStatus {
  autostartEnabled: boolean | null;
  autostartError: string | null;
  shortcutErrors: Record<string, string>;
  autoSelection: { active: boolean; error: string | null };
}

export interface Settings {
  queryFromLanguage: Language;
  queryToLanguage: Language;
  firstLanguage: Language;
  secondLanguage: Language;
  appearanceType: AppearanceType;
  fontSizeOptionIndex: number;
  autoPlayAudio: boolean;
  pronunciationAccent: string;
  defaultTtsServiceType: TtsServiceType;
  preferYoudaoTtsForEnglishWord: boolean;
  languageDetectOptimize: LanguageDetectOptimize;
  hideMainWindow: boolean;
  clearQueryWhenInputTranslate: boolean;
  selectQueryTextWhenWindowActivate: boolean;
  enableMarkdownRendering: boolean;
  autoSelectAllTextFieldText: boolean;

  autoQuerySelectedText: boolean;
  autoQueryPastedText: boolean;
  autoQueryOcrText: boolean;
  autoQueryWhenTextChanged: boolean;

  autoCopySelectedText: boolean;
  autoCopyOcrText: boolean;
  autoCopyFirstTranslatedText: boolean;
  keepPrevResultWhenSelectedTextIsEmpty: boolean;

  autoShowQueryIcon: boolean;
  autoShowQueryIconExcludedLanguage: Language;
  autoShowQueryIconMinTextLength: number;
  clickQuery: boolean;

  fixedWindowPosition: ShowWindowPosition;
  miniWindowPosition: ShowWindowPosition;
  mouseSelectTranslateWindowType: WindowType;
  shortcutSelectTranslateWindowType: WindowType;
  /**
   * `WindowConfigurationKey.swift` stores one key per window type
   * (`EZConfiguration_InputFieldCellVisible_Window0_Key`, …), so each query
   * window decides for itself whether it shows its input field and its
   * language row. Both default to `true` upstream.
   */
  inputFieldCellVisibleMain: boolean;
  inputFieldCellVisibleMini: boolean;
  inputFieldCellVisibleFixed: boolean;
  selectLanguageCellVisibleMain: boolean;
  selectLanguageCellVisibleMini: boolean;
  selectLanguageCellVisibleFixed: boolean;
  pinWindowWhenDisplayed: boolean;
  maxWindowHeightPercentage: number;
  intelligentQueryMode: Partial<Record<WindowType, boolean>>;

  automaticWordSegmentation: boolean;
  automaticallyRemoveCodeCommentSymbols: boolean;
  replaceNewlineWithSpace: boolean;
  enableRemoveBooksExcerptInfo: boolean;
  enableForceGetSelectedText: boolean;
  enableOcrTextNormalization: boolean;

  hideMenuBarIcon: boolean;
  selectedMenuBarIcon: string;
  showGoogleQuickLink: boolean;
  showEudicQuickLink: boolean;
  /** `EZConfiguration_kShowAppleDictionaryLinkKey` — no target on Windows, kept for round-tripping. */
  showAppleDictionaryQuickLink: boolean;
  showQuickActionButton: boolean;

  showOcrMenuItems: boolean;
  enableYoudaoOcr: boolean;
  isScreenshotTipLayerHidden: boolean;

  enableBetaFeature: boolean;
  enableHttpServer: boolean;
  httpPort: string;
  allowAnalytics: boolean;
  allowCrashLog: boolean;

  launchAtStartup: boolean;
  firstLaunch: boolean;

  deepLAuth: string;
  deepLTranslateEndPoint: string;
  deepLTranslation: DeepLAPIUsagePriority;
  deepLWebAppVersionCache: string;
  youdaoWebApiKey: string;
  bingCookie: string;
  niuTransApiKey: string;
  caiyunToken: string;
  tencentSecretId: string;
  tencentSecretKey: string;
  aliAccessKeyId: string;
  aliAccessKeySecret: string;
  aliServiceApiType: ServiceApiType;
  baiduAppId: string;
  baiduSecretKey: string;
  baiduServiceApiType: ServiceApiType;
  volcanoAccessKeyId: string;
  volcanoSecretAccessKey: string;
  doubaoApiKey: string;
  doubaoModel: string;

  services: Record<string, ServiceConfig>;
  serviceOrder: string[];

  shortcuts: Record<string, string>;

  queryHistory: QueryRecord[];
  favorites: QueryRecord[];

  mainWindowWidth: number;
  mainWindowHeight: number;
  fontSizeRatioIndex: number;
}

/** A deep partial, for `updateSettings` / `updateServiceConfig`. */
export type DeepPartial<T> = {
  [K in keyof T]?: T[K] extends (infer U)[]
    ? U[]
    : T[K] extends object
      ? DeepPartial<T[K]>
      : T[K];
};

export interface ServiceDescriptor {
  identifier: string;
  serviceType: ServiceType;
  name: string;
  nameKey: string;
  apiKeyRequirement: ApiKeyRequirement;
  isStream: boolean;
  link: string | null;
  supportedQueryType: number;
  configurableIntelligentQuery: boolean;
  config: ServiceConfig;
  requiresApiKey: boolean;
  hasApiKey: boolean;
  /** False for the two services that wrap macOS-only frameworks. */
  available: boolean;
  allowsMultipleInstances: boolean;
}

export interface ServiceValidation {
  success: boolean;
  message: string;
  validationNote?: string | null;
}

export interface AppInfo {
  name: string;
  version: string;
  platform: string;
  dataDir: string;
}

// ---------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------

export interface QueryRequest {
  text: string;
  requestId?: string;
  from?: Language;
  to?: Language;
  windowType?: "main" | "mini" | "fixed";
}

export interface TtsRequest {
  text: string;
  language?: Language;
  from?: Language;
  to?: Language;
  isEnglishWord?: boolean;
  url?: string | null;
}

export interface OcrLine {
  text: string;
  /** x, y, width, height */
  boundingBox: [number, number, number, number];
}

export interface OcrOutput {
  text: string;
  lines: OcrLine[];
  language: Language | null;
  confidence: number | null;
}

export interface ScreenshotOutput {
  output: OcrOutput | null;
  restoreWindow: boolean;
}

export interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** An entry in `SHORTCUT_ACTIONS` on the Rust side. */
export interface ShortcutActionDef {
  id: string;
  titleKey: string;
  title: string;
  global: boolean;
}
