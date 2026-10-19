/**
 * The configuration panel for one service.
 *
 * Per-service configuration writes through `api.updateServiceConfig` — text
 * fields on a 300 ms debounce, toggles and pickers immediately — and each of
 * those writes ends with a descriptor reload so the panel shows what the
 * backend actually stored.
 *
 * A handful of services keep their credentials in flat, top-level `Settings`
 * fields instead (upstream's `Defaults` keys such as `EZBaiduAppId`), so those
 * write through the settings patch — see `SERVICE_CREDENTIAL_FIELDS`.
 */

import { useCallback, useMemo, useState } from "react";

import * as api from "../api";
import type {
  DeepLAPIUsagePriority,
  DeepPartial,
  ReasoningEffort,
  ServiceConfig,
  ServiceDescriptor,
  ServiceUsageStatus,
  ServiceValidation,
  Settings,
} from "../types";
import {
  Banner,
  Icon,
  QUERY_TYPE_DICTIONARY,
  QUERY_TYPE_SENTENCE,
  QUERY_TYPE_TRANSLATION,
  Row,
  SecretField,
  Section,
  Select,
  Slider,
  TextField,
  Toggle,
  errorMessage,
  serviceName,
  serviceMonogram,
  type SelectOption,
} from "./components";

const USAGE_OPTIONS: readonly SelectOption<ServiceUsageStatus>[] = [
  { value: "0", label: "默认" },
  { value: "1", label: "始终关闭" },
  { value: "2", label: "始终开启" },
];

const REASONING_OPTIONS: readonly SelectOption<ReasoningEffort>[] = [
  { value: "off", label: "关闭" },
  { value: "high", label: "高" },
  { value: "max", label: "最高" },
];

const API_KEY_NOTES: Record<string, string> = {
  none: "该服务无需 API Key。",
  builtIn: "使用内置密钥，无需填写。",
  agentCli: "本机登录对应 CLI 后即可使用，无需填写密钥。",
};

/** `DeepLService.swift`'s `StaticPickerCell` over `DeepLAPIUsagePriority`. */
const DEEPL_PRIORITY_OPTIONS: readonly SelectOption<DeepLAPIUsagePriority>[] = [
  { value: "webFirst", label: "优先使用 Web API" },
  { value: "authKeyFirst", label: "优先使用 Auth Key" },
  { value: "authKeyOnly", label: "仅使用 Auth Key" },
];

/** `DoubaoService.defaultModelIdentifier` (`doubao.rs` `DEFAULT_MODEL`). */
const DOUBAO_DEFAULT_MODEL = "doubao-seed-translation-250915";

/**
 * Services upstream authenticates with a key vendored in the app.
 *
 * This port ships no such key, so they are configured with the user's own
 * `services.<id>.apiKey` instead — see `BuiltInAiService` (`llm/openai.rs`) and
 * `missing_built_in_key` (`llm/ai_tool.rs`), whose error messages say exactly
 * that. Their `ApiKeyRequirement` is still `builtIn`, so the descriptor's
 * `requiresApiKey` is false and the generic row has to be enabled by hand.
 */
const SERVICES_WITHOUT_BUILT_IN_KEY = new Set(["BuiltInAI", "Polishing", "Summary"]);

/**
 * The flat `Settings` keys a service's own configuration view edits.
 *
 * These mirror upstream's `Defaults` keys one-for-one (`EZBaiduAppId` →
 * `baiduAppId`) and are read by the Rust services straight off `Settings` — a
 * value written into `services.<identifier>.apiKey` never reaches them. The
 * union is closed so a typo cannot silently write a field that is not part of
 * the settings document.
 */
type FlatCredentialKey =
  | "baiduAppId"
  | "baiduSecretKey"
  | "tencentSecretId"
  | "tencentSecretKey"
  | "aliAccessKeyId"
  | "aliAccessKeySecret"
  | "volcanoAccessKeyId"
  | "volcanoSecretAccessKey"
  | "doubaoApiKey"
  | "caiyunToken"
  | "niuTransApiKey"
  | "youdaoWebApiKey"
  | "deepLAuth"
  | "deepLTranslateEndPoint"
  | "bingCookie";

interface CredentialField {
  key: FlatCredentialKey;
  /** Upstream's label, e.g. `service.configuration.baidu.app_id.title`. */
  label: string;
  placeholder: string;
  description: string;
  /** False when the service can answer without this field. */
  required: boolean;
  /** Upstream's `SecureInputCell` (masked) versus `InputCell` (plain). */
  masked: boolean;
  /** Identifiers and addresses read better monospace. */
  monospace?: boolean;
}

/**
 * The services whose credentials live outside `ServiceConfig`, keyed by
 * `ServiceType::raw()`. Everything else keeps the generic API Key row, which
 * is what the LLM services actually read.
 */
const SERVICE_CREDENTIAL_FIELDS: Record<string, readonly CredentialField[]> = {
  // BaiduService.swift:24-26
  Baidu: [
    {
      key: "baiduAppId",
      label: "App ID",
      placeholder: "百度翻译 App ID",
      description: "百度翻译开放平台的 App ID",
      required: true,
      masked: true,
      monospace: true,
    },
    {
      key: "baiduSecretKey",
      label: "Secret Key",
      placeholder: "百度翻译密钥",
      description: "百度翻译开放平台的密钥",
      required: true,
      masked: true,
    },
  ],
  // TencentService.swift:48
  Tencent: [
    {
      key: "tencentSecretId",
      label: "Secret ID",
      placeholder: "腾讯云 SecretId",
      description: "腾讯云访问密钥 SecretId",
      required: true,
      masked: true,
      monospace: true,
    },
    {
      key: "tencentSecretKey",
      label: "Secret Key",
      placeholder: "腾讯云 SecretKey",
      description: "腾讯云访问密钥 SecretKey",
      required: true,
      masked: true,
    },
  ],
  // AliService.swift
  Alibaba: [
    {
      key: "aliAccessKeyId",
      label: "AccessKey ID",
      placeholder: "阿里云 AccessKey ID",
      description: "阿里云访问密钥 ID",
      required: true,
      masked: true,
      monospace: true,
    },
    {
      key: "aliAccessKeySecret",
      label: "AccessKey Secret",
      placeholder: "阿里云 AccessKey Secret",
      description: "阿里云访问密钥 Secret",
      required: true,
      masked: true,
    },
  ],
  // VolcanoService.swift:165-167
  Volcano: [
    {
      key: "volcanoAccessKeyId",
      label: "Access Key ID",
      placeholder: "火山引擎 Access Key ID",
      description: "火山引擎访问密钥 ID",
      required: true,
      masked: true,
      monospace: true,
    },
    {
      key: "volcanoSecretAccessKey",
      label: "Secret Access Key",
      placeholder: "火山引擎 Secret Access Key",
      description: "火山引擎访问密钥 Secret",
      required: true,
      masked: true,
    },
  ],
  // DoubaoService.swift:61-69 — the model input is rendered separately, in the
  // model section, because it is not a secret.
  Doubao: [
    {
      key: "doubaoApiKey",
      label: "API Key",
      placeholder: "火山方舟 API Key",
      description: "火山方舟（豆包）API Key",
      required: true,
      masked: true,
    },
  ],
  // CaiyunService.swift:42
  Caiyun: [
    {
      key: "caiyunToken",
      label: "Token",
      placeholder: "彩云小译 Token",
      description: "彩云小译 Token",
      required: true,
      masked: true,
    },
  ],
  // NiuTransService.swift:26 — upstream ships a built-in key, this port does not.
  NiuTrans: [
    {
      key: "niuTransApiKey",
      label: "API Key",
      placeholder: "小牛翻译 API Key",
      description: "小牛翻译 API Key",
      required: true,
      masked: true,
    },
  ],
  // YoudaoService.swift — upstream vendors the web API key in source; this port
  // does not ship it, so the user supplies their own (`youdao.rs` `default_key`).
  Youdao: [
    {
      key: "youdaoWebApiKey",
      label: "Web API Key",
      placeholder: "有道网页版密钥",
      description: "该版本不内置有道网页版签名密钥，未填写时有道无法查询",
      required: true,
      masked: true,
    },
  ],
  // DeepLService.swift — auth key, endpoint and priority picker. The key is
  // optional: the web transport works without one, and the priority decides
  // whether the official API is used at all.
  DeepL: [
    {
      key: "deepLAuth",
      label: "Auth Key",
      placeholder: "DeepL Auth Key",
      description: "官方 API 密钥，留空时仅使用网页接口",
      required: false,
      masked: true,
    },
    {
      key: "deepLTranslateEndPoint",
      label: "API 请求地址",
      placeholder: "https://api-free.deepl.com/v2/translate",
      description: "官方 API 地址，留空时按密钥是否以 :fx 结尾自动选择",
      required: false,
      masked: false,
      monospace: true,
    },
  ],
  // BingService.swift — the cookie only accompanies the scraped session token,
  // which the service obtains on its own.
  Bing: [
    {
      key: "bingCookie",
      label: "Cookie Key",
      placeholder: "可选",
      description: "可选，随必应请求发送的 Cookie",
      required: false,
      masked: true,
    },
  ],
};

export function ServiceDetail({
  service,
  settings,
  patch,
  onBack,
  onChanged,
}: {
  service: ServiceDescriptor;
  /** The live settings document, for the flat per-service credential fields. */
  settings: Settings;
  /** The settings store's write, for the same fields. */
  patch: (patch: DeepPartial<Settings>) => Promise<void>;
  onBack: () => void;
  onChanged: () => Promise<void>;
}) {
  const config = service.config;
  const link = service.link;
  const [writeError, setWriteError] = useState<string | null>(null);
  const [validating, setValidating] = useState(false);
  const [validation, setValidation] = useState<ServiceValidation | null>(null);
  const [validationError, setValidationError] = useState<string | null>(null);
  const [fetchingModels, setFetchingModels] = useState(false);
  const [modelsMessage, setModelsMessage] = useState<string | null>(null);
  const [modelsError, setModelsError] = useState<string | null>(null);
  const [savingIntelligent, setSavingIntelligent] = useState(false);

  const write = useCallback(
    async (patchValue: DeepPartial<ServiceConfig>) => {
      try {
        await api.updateServiceConfig(service.identifier, patchValue);
        setWriteError(null);
        await onChanged();
      } catch (cause) {
        setWriteError(errorMessage(cause));
      }
    },
    [service.identifier, onChanged],
  );

  // Credentials that live in the settings document rather than in
  // `services.<identifier>` go through the settings store instead, so every
  // tab sees the write.
  const writeSettings = useCallback(
    async (patchValue: DeepPartial<Settings>) => {
      try {
        await patch(patchValue);
        setWriteError(null);
      } catch (cause) {
        setWriteError(errorMessage(cause));
      }
    },
    [patch],
  );

  const isDoubao = service.serviceType === "Doubao";
  const credentialFields = SERVICE_CREDENTIAL_FIELDS[service.serviceType] ?? null;
  const usesGenericApiKey =
    service.requiresApiKey || SERVICES_WITHOUT_BUILT_IN_KEY.has(service.serviceType);
  const missingCredential =
    credentialFields?.some(
      (field) => field.required && settings[field.key].trim() === "",
    ) ?? false;

  // The model list is only meaningful for services that talk to a model API:
  // the streaming ones, plus anything that already carries a model list.
  const supportsModels =
    service.isStream ||
    config.validModels.length > 0 ||
    config.supportedModels.length > 0 ||
    config.endpoint.trim() !== "";

  const modelOptions = useMemo<SelectOption<string>[]>(() => {
    const source =
      config.validModels.length > 0 ? config.validModels : config.supportedModels;
    const ids = new Set(source);
    const selectedModel = config.model.trim();
    if (selectedModel !== "") {
      ids.add(selectedModel);
    }
    return [...ids].map((id) => ({ value: id, label: id }));
  }, [config.validModels, config.supportedModels, config.model]);

  const runValidation = async () => {
    setValidating(true);
    setValidation(null);
    setValidationError(null);
    try {
      const result = await api.validateService(service.identifier);
      setValidation(result);
      // Validation may have turned streaming off as a side effect.
      await onChanged();
    } catch (cause) {
      setValidationError(errorMessage(cause));
    } finally {
      setValidating(false);
    }
  };

  const fetchModels = async () => {
    setFetchingModels(true);
    setModelsMessage(null);
    setModelsError(null);
    try {
      const ids = await api.fetchServiceModels(service.identifier);
      if (ids.length === 0) {
        setModelsMessage("服务返回了空的模型列表。");
      } else {
        await write({ validModels: ids, supportedModels: ids });
        setModelsMessage(`已获取 ${ids.length} 个模型。`);
      }
    } catch (cause) {
      setModelsError(errorMessage(cause));
    } finally {
      setFetchingModels(false);
    }
  };

  const capability = (
    label: string,
    bit: number,
    key: "translation" | "dictionary" | "sentence",
  ) => (
    <Row
      label={label}
      description={
        (service.supportedQueryType & bit) === 0
          ? "该服务不支持此查询类型"
          : undefined
      }
      disabled={(service.supportedQueryType & bit) === 0}
    >
      <Toggle
        checked={config[key]}
        label={label}
        onChange={(value) => void write({ [key]: value })}
      />
    </Row>
  );

  const showValidation = validating || validation !== null || validationError !== null;
  const validationKind: "info" | "success" | "error" = validating
    ? "info"
    : validationError !== null || validation?.success !== true
      ? "error"
      : "success";
  const validationText = validating
    ? "正在校验…"
    : validationError !== null
      ? `校验失败：${validationError}`
      : validation !== null
        ? `${validation.message}${
            validation.validationNote ? `（${validation.validationNote}）` : ""
          }`
        : "";
  const dismissValidation = () => {
    setValidation(null);
    setValidationError(null);
  };

  return (
    <>
      <div className="ezy-detail-header">
        <button type="button" className="ez-btn" onClick={onBack}>
          <Icon name="back" size={12} />
          返回
        </button>
        <span className="ezy-service-icon">{serviceMonogram(service)}</span>
        <div className="ezy-detail-heading">
          <span className="ezy-detail-title">{serviceName(service)}</span>
          <span className="ezy-detail-sub ez-selectable">{service.identifier}</span>
        </div>
        <div className="ezy-detail-actions">
          {link !== null && link !== "" && (
            <button
              type="button"
              className="ez-btn"
              onClick={() => void api.openLink(link)}
            >
              <Icon name="external" size={12} />
              打开官网
            </button>
          )}
          <button
            type="button"
            className="ez-btn ez-btn-primary"
            disabled={validating}
            onClick={() => void runValidation()}
          >
            {validating ? <span className="ez-spinner" /> : <Icon name="check" size={12} />}
            校验
          </button>
        </div>
      </div>

      {writeError !== null && (
        <Banner kind="error" onDismiss={() => setWriteError(null)}>
          保存失败：{writeError}
        </Banner>
      )}

      {showValidation && (
        <Banner kind={validationKind} onDismiss={validating ? undefined : dismissValidation}>
          {validationText}
        </Banner>
      )}

      <Section title="基本">
        {service.allowsMultipleInstances && (
          <Row label="名称" description="用于区分同一服务的多个实例">
            <TextField
              value={config.name}
              placeholder={serviceName(service)}
              label="名称"
              onChange={(value) => void write({ name: value })}
            />
          </Row>
        )}
        <Row label="启用" description="关闭后该服务不参与查询">
          <Toggle
            checked={config.enabled}
            label="启用"
            onChange={(value) => void write({ enabled: value })}
          />
        </Row>
        <Row label="使用状态" description="始终关闭 / 始终开启会覆盖自动显示逻辑">
          <Select
            value={config.usageStatus}
            options={USAGE_OPTIONS}
            label="使用状态"
            onChange={(value) => void write({ usageStatus: value })}
          />
        </Row>
        {capability("翻译", QUERY_TYPE_TRANSLATION, "translation")}
        {capability("词典", QUERY_TYPE_DICTIONARY, "dictionary")}
        {capability("句子", QUERY_TYPE_SENTENCE, "sentence")}
      </Section>

      {service.configurableIntelligentQuery && (
        <Section
          title="智能查询参与类型"
          description="仅在查询窗口开启智能查询时生效；不改变上方的翻译、词典或句子输出模式。全部关闭则不自动查询本服务。"
        >
          {([
            ["普通文本", QUERY_TYPE_TRANSLATION],
            ["单词", QUERY_TYPE_DICTIONARY],
            ["英文句子", QUERY_TYPE_SENTENCE],
          ] as const).map(([label, bit]) => (
            <Row key={bit} label={label}>
              <Toggle
                checked={((config.intelligentQueryTextType ?? 7) & bit) !== 0}
                disabled={savingIntelligent}
                label={`智能查询${label}`}
                onChange={async (value) => {
                  setSavingIntelligent(true);
                  const current = config.intelligentQueryTextType ?? 7;
                  try {
                    await write({ intelligentQueryTextType: value ? current | bit : current & ~bit });
                  } finally {
                    setSavingIntelligent(false);
                  }
                }}
              />
            </Row>
          ))}
        </Section>
      )}

      <Section title="认证">
        {credentialFields !== null ? (
          <>
            {missingCredential && (
              <Row label="尚未填写完整" description="该服务必须填写下列密钥后才能查询">
                <span className="ezy-caution" title="缺少密钥">
                  <Icon name="warning" />
                </span>
              </Row>
            )}
            {credentialFields.map((field) => {
              // Masked exactly where upstream uses a `SecureInputCell`.
              const Field = field.masked ? SecretField : TextField;
              return (
                <Row key={field.key} label={field.label} description={field.description}>
                  <Field
                    value={settings[field.key]}
                    placeholder={field.placeholder}
                    label={field.label}
                    monospace={field.monospace}
                    width={280}
                    onChange={(value) => void writeSettings({ [field.key]: value })}
                  />
                </Row>
              );
            })}
            {service.serviceType === "DeepL" && (
              <Row label="API 使用优先级" description="决定官方 Auth Key 与网页接口的先后顺序">
                <Select
                  value={settings.deepLTranslation}
                  options={DEEPL_PRIORITY_OPTIONS}
                  label="API 使用优先级"
                  onChange={(value) => void writeSettings({ deepLTranslation: value })}
                />
              </Row>
            )}
          </>
        ) : usesGenericApiKey ? (
          <>
            {!service.hasApiKey && (
              <Row label="尚未填写 API Key" description="该服务必须填写密钥后才能查询">
                <span className="ezy-caution" title="缺少 API Key">
                  <Icon name="warning" />
                </span>
              </Row>
            )}
            <Row label="API Key" description="仅保存在本机设置文件中">
              <SecretField
                value={config.apiKey}
                placeholder="粘贴 API Key"
                label="API Key"
                width={280}
                onChange={(value) => void write({ apiKey: value })}
              />
            </Row>
          </>
        ) : (
          <Row
            label="API Key"
            description={API_KEY_NOTES[service.apiKeyRequirement] ?? "该服务无需填写密钥。"}
          >
            <span className="ez-badge">无需填写</span>
          </Row>
        )}
        {/* None of the services with their own credential block reads
            `config.endpoint`; their endpoints are constants or flat fields. */}
        {supportsModels && credentialFields === null && (
          <Row label="API 端点" description="留空时使用该服务的默认端点">
            <TextField
              value={config.endpoint}
              placeholder="https://…/v1/chat/completions"
              label="API 端点"
              monospace
              width={320}
              onChange={(value) => void write({ endpoint: value })}
            />
          </Row>
        )}
      </Section>

      {supportsModels && (
        <Section title="模型与推理">
          <Row
            label="模型"
            description={isDoubao ? `留空时使用默认模型 ${DOUBAO_DEFAULT_MODEL}` : undefined}
          >
            <div className="ezy-inline">
              {isDoubao ? (
                // Doubao reads the flat `doubao_model` (`doubao.rs` `model()`),
                // never `services.Doubao.model`, so it gets its own input —
                // `DoubaoService.swift:61-69` renders the same field.
                <TextField
                  value={settings.doubaoModel}
                  placeholder={DOUBAO_DEFAULT_MODEL}
                  label="模型"
                  monospace
                  width={240}
                  onChange={(value) => void writeSettings({ doubaoModel: value })}
                />
              ) : modelOptions.length > 0 ? (
                <Select
                  value={config.model}
                  options={modelOptions}
                  label="模型"
                  width={240}
                  onChange={(value) => void write({ model: value })}
                />
              ) : (
                <TextField
                  value={config.model}
                  placeholder="模型 ID"
                  label="模型"
                  monospace
                  width={240}
                  onChange={(value) => void write({ model: value })}
                />
              )}
              {/* The fetch goes through `config.apiKey` / `config.endpoint`,
                  which the services with their own credential block never
                  populate, so it is only offered where it can work. */}
              {credentialFields === null && (
                <button
                  type="button"
                  className="ez-btn"
                  disabled={fetchingModels}
                  onClick={() => void fetchModels()}
                >
                  {fetchingModels ? (
                    <span className="ez-spinner" />
                  ) : (
                    <Icon name="refresh" size={12} />
                  )}
                  获取模型列表
                </button>
              )}
            </div>
          </Row>
          {modelsError !== null && (
            <Banner kind="error" onDismiss={() => setModelsError(null)}>
              获取模型列表失败：{modelsError}
            </Banner>
          )}
          {modelsMessage !== null && <p className="ezy-hint">{modelsMessage}</p>}
          {/* Doubao's request body carries no temperature and no reasoning
              effort — `DoubaoService.buildRequestBody` sends only model,
              stream and input — and upstream's panel for it shows the API key
              and the model alone, so these three would be controls that write
              a field the service never reads. Streaming is kept because this
              port's `doubao.rs` does branch on it. */}
          {!isDoubao && (
            <Row label="温度" description="越低越稳定，越高越发散">
              <Slider
                value={config.temperature}
                min={0}
                max={2}
                step={0.1}
                label="温度"
                format={(value) => value.toFixed(1)}
                onChange={(value) => void write({ temperature: value })}
              />
            </Row>
          )}
          <Row label="流式输出" description="边生成边显示结果">
            <Toggle
              checked={config.enableStreaming}
              label="流式输出"
              onChange={(value) => void write({ enableStreaming: value })}
            />
          </Row>
          {!isDoubao && (
            <Row label="推理强度" description="仅对支持思考的模型生效">
              <Select
                value={config.reasoningEffort}
                options={REASONING_OPTIONS}
                label="推理强度"
                onChange={(value) => void write({ reasoningEffort: value })}
              />
            </Row>
          )}
          <Row label="隐藏 thinking 内容" description="从结果中移除思考过程">
            <Toggle
              checked={config.hideThinkTag}
              label="隐藏 thinking 内容"
              onChange={(value) => void write({ hideThinkTag: value })}
            />
          </Row>
        </Section>
      )}

      {supportsModels && !isDoubao && (
        <Section title="自定义提示词">
          <Row label="启用自定义提示词">
            <Toggle
              checked={config.enableCustomPrompt}
              label="启用自定义提示词"
              onChange={(value) => void write({ enableCustomPrompt: value })}
            />
          </Row>
          <Row label="系统提示词" stacked>
            <TextField
              value={config.systemPrompt}
              multiline
              rows={4}
              disabled={!config.enableCustomPrompt}
              placeholder="You are a professional translation engine…"
              label="系统提示词"
              onChange={(value) => void write({ systemPrompt: value })}
            />
          </Row>
          <Row label="用户提示词" stacked description="使用 {{query}} 占位待翻译文本">
            <TextField
              value={config.userPrompt}
              multiline
              rows={4}
              disabled={!config.enableCustomPrompt}
              placeholder="{{query}}"
              label="用户提示词"
              onChange={(value) => void write({ userPrompt: value })}
            />
          </Row>
        </Section>
      )}
    </>
  );
}
