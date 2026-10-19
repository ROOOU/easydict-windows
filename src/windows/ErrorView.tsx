/**
 * The error body of a card.
 *
 * Compact and red-tinted, with the service's own message first, then the raw
 * upstream payload (when there is one) in a monospace block, and a retry
 * affordance. The macOS app additionally offers "open the service page" for
 * missing-key errors; the backend hands us that URL in `promptUrl`.
 */

import * as api from "../api";
import type { ErrorType, QueryError } from "../types";
import { ErrorIcon, ExternalIcon, RetryIcon } from "./icons";

export interface ErrorViewProps {
  error: QueryError;
  onRetry: () => void;
}

/** Mirrors the localised descriptions in `EZError`. */
const DEFAULT_MESSAGES: Record<ErrorType, string> = {
  unknown: "未知错误",
  api: "请求接口失败",
  parameter: "请求参数错误",
  unsupportedLanguage: "不支持的语言",
  missingSecretKey: "缺少密钥，请在设置中填写",
  noResult: "未找到结果",
  timeout: "请求超时",
  unsupportedQueryType: "不支持的查询类型",
  unsupportedServiceType: "不支持的服务类型",
  contentTypeMismatch: "查询内容与服务类型不匹配",
};

export function ErrorView({ error, onRetry }: ErrorViewProps) {
  const message = error.message?.trim() || DEFAULT_MESSAGES[error.type];
  const data = error.errorDataMessage?.trim() ?? "";
  const promptUrl = error.promptUrl?.trim() ?? "";

  return (
    <div className="ez-error">
      <div className="ez-error-row">
        <span className="ez-error-icon" aria-hidden="true">
          <ErrorIcon size={15} />
        </span>
        <span className="ez-error-message ez-selectable">{message}</span>
        <button type="button" className="ez-btn ez-error-retry" onClick={onRetry}>
          <RetryIcon size={13} />
          重试
        </button>
      </div>

      {data.length > 0 ? <pre className="ez-error-data ez-selectable">{data}</pre> : null}

      {promptUrl.length > 0 ? (
        <button
          type="button"
          className="ez-btn ez-error-prompt"
          onClick={() => void api.openLink(promptUrl)}
        >
          <ExternalIcon size={13} />
          {error.promptTitle?.trim() || "打开服务页面"}
        </button>
      ) : null}
    </div>
  );
}
