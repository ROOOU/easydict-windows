// Evaluates one expression in every WebView2 page of the running app.
//
//     node tools/dev/cdp-inspect.mjs "document.body.innerText"
//
// Requires the app to be started with
// `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222`.
const endpoint = process.env.CDP_ENDPOINT ?? "http://127.0.0.1:9222";
const expression = process.argv[2] ?? "document.body.innerText";

const targets = await (await fetch(`${endpoint}/json/list`)).json();
const pages = targets.filter((target) => target.type === "page");
if (pages.length === 0) {
  console.log("no pages found; is the app running with the debugging port?");
  process.exit(1);
}

for (const page of pages) {
  const socket = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => {
    socket.addEventListener("open", resolve, { once: true });
    socket.addEventListener("error", reject, { once: true });
  });
  const result = await new Promise((resolve) => {
    const id = 1;
    socket.addEventListener("message", (event) => {
      const message = JSON.parse(event.data);
      if (message.id === id) resolve(message.result);
    });
    socket.send(JSON.stringify({
      id,
      method: "Runtime.evaluate",
      params: { expression, returnByValue: true, awaitPromise: true },
    }));
  });
  socket.close();
  const value = result?.result?.value ?? result?.exceptionDetails?.text ?? "<no result>";
  console.log(`--- ${page.title} ${page.url} ---`);
  console.log(typeof value === "string" ? value : JSON.stringify(value));
}
