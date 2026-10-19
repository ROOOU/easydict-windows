#!/usr/bin/env node
/**
 * A minimal OpenAI-compatible endpoint, for verifying the "流式输出" toggle by
 * hand — without an API key and without guessing what the provider did.
 *
 *     node tools/mock-llm-server.mjs               # answers in the shape asked for
 *     node tools/mock-llm-server.mjs --json-only   # answers JSON even to a streaming request
 *     node tools/mock-llm-server.mjs --sse-only    # the other way round
 *
 * Then in the app: 设置 → 服务 → CustomOpenAI
 *     认证 → API 端点   http://127.0.0.1:8321/v1/chat/completions
 *     （API Key 随便填一个非空值，否则服务会在发出请求前就报缺密钥）
 *
 * Every request is printed with the `stream` flag it carried and the shape it
 * was answered with, so the toggle's effect is visible from both sides.
 */

import http from "node:http";

const PORT = Number(process.env.PORT ?? 8321);
const args = new Set(process.argv.slice(2));
const mode = args.has("--json-only") ? "json" : args.has("--sse-only") ? "sse" : "echo";

/** One OpenAI-compatible streaming delta. */
function chunk(content) {
  return JSON.stringify({
    id: "1",
    object: "chat.completion.chunk",
    created: 0,
    model: "mock-model",
    choices: [{ index: 0, delta: { content }, finish_reason: null }],
  });
}

/** One OpenAI-compatible non-streaming completion. */
function completion(content) {
  return JSON.stringify({
    id: "1",
    object: "chat.completion",
    created: 0,
    model: "mock-model",
    choices: [
      {
        index: 0,
        message: { role: "assistant", content },
        finish_reason: "stop",
      },
    ],
    usage: { prompt_tokens: 1, completion_tokens: 1, total_tokens: 2 },
  });
}

const server = http.createServer((req, res) => {
  let raw = "";
  req.on("data", (part) => {
    raw += part;
  });
  req.on("end", () => {
    let wantsStream = null;
    try {
      wantsStream = JSON.parse(raw).stream === true;
    } catch {
      // Left null: the body was not JSON, which is itself worth seeing.
    }

    // `echo` follows the request; the two overrides deliberately do not, so the
    // mismatch the toggle is supposed to prevent can be reproduced on demand.
    const stream = mode === "echo" ? wantsStream === true : mode === "sse";

    console.log(
      `${new Date().toLocaleTimeString()}  ${req.method} ${req.url}  ` +
        `stream=${wantsStream}  ->  ${stream ? "text/event-stream" : "application/json"}`,
    );

    if (stream) {
      const answer = "流式响应 ✅";
      res.writeHead(200, {
        "Content-Type": "text/event-stream",
        "Cache-Control": "no-cache",
        Connection: "keep-alive",
      });
      // Two deltas, so a partial render is observable before the stream ends.
      res.write(`data: ${chunk(answer.slice(0, 2))}\n\n`);
      res.write(`data: ${chunk(answer.slice(2))}\n\n`);
      res.write("data: [DONE]\n\n");
      res.end();
      return;
    }

    res.writeHead(200, { "Content-Type": "application/json" });
    res.end(completion("非流式响应 ✅"));
  });
});

server.listen(PORT, "127.0.0.1", () => {
  console.log(`mock LLM endpoint on http://127.0.0.1:${PORT}/v1/chat/completions`);
  console.log(
    mode === "echo"
      ? "mode: echo — replies in whatever shape the request asked for"
      : `mode: ${mode}-only — always replies as ${mode === "sse" ? "SSE" : "JSON"}`,
  );
});
