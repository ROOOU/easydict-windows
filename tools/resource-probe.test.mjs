import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { join } from "node:path";
import test from "node:test";

/**
 * PowerShell hosts to try, most capable first.
 *
 * `pwsh` is what a developer with PowerShell 7 installed gets, but Windows ships
 * only `powershell.exe`, and a machine without PowerShell 7 on PATH still has to
 * be able to run the collector's self-test. The script itself is written to work
 * on both hosts; see the .NET Framework notes in `tools/dev/resource-probe.ps1`.
 */
function hosts() {
  return [
    process.env.PSHOME && join(process.env.PSHOME, "pwsh.exe"),
    "pwsh",
    join(process.env.ProgramFiles ?? "C:\\Program Files", "PowerShell", "7", "pwsh.exe"),
    "powershell.exe",
    join(
      process.env.SystemRoot ?? "C:\\Windows",
      "System32",
      "WindowsPowerShell",
      "v1.0",
      "powershell.exe",
    ),
  ].filter(Boolean);
}

function run(host, args) {
  return spawnSync(host, ["-NoProfile", "-ExecutionPolicy", "Bypass", ...args], {
    encoding: "utf8",
  });
}

function availableHost() {
  for (const host of hosts()) {
    const result = run(host, ["-Command", "exit 0"]);
    if (!result.error && result.status === 0) return host;
  }
  return null;
}

test("Windows resource collector rejects PID reuse and invalid CPU samples", { skip: process.platform !== "win32" }, (t) => {
  const host = availableHost();
  if (!host) {
    t.skip(`no PowerShell host found; tried ${hosts().join(", ")}`);
    return;
  }
  const result = run(host, ["-File", "tools/dev/resource-probe.ps1", "-SelfTest"]);
  assert.equal(result.status, 0, `${host}: ${result.stdout ?? ""}${result.stderr ?? ""}${result.error ?? ""}`);
  assert.match(result.stdout, /self-test passed/);
});
