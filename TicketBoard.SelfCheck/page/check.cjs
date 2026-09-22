// Сквозная проверка страницы моста в Chromium: настоящий мост (SelfCheck bridge) + поддельный Интрасервис
// + подменённый api.anthropic.com. Запуск — run.sh; аргументы: ключ моста и тема (light | dark).
const { chromium } = require("playwright");
const assert = require("node:assert/strict");
const os = require("node:os");
const path = require("node:path");

const KEY = process.argv[2];
const BASE = "http://127.0.0.1:47822";

function chunks(s, n) { const out = []; for (let i = 0; i < s.length; i += n) out.push(s.slice(i, i + n)); return out; }

let msgNo = 0;
function sse(m) {
  const ev = [];
  const push = (type, data) => ev.push(`event: ${type}\ndata: ${JSON.stringify({ type, ...data })}\n\n`);
  push("message_start", { message: { id: `msg_${++msgNo}`, type: "message", role: "assistant", model: "claude-opus-5", content: [],
    stop_reason: null, stop_sequence: null,
    usage: { input_tokens: m.usage.input, output_tokens: 1, cache_creation_input_tokens: m.usage.cw ?? 0, cache_read_input_tokens: m.usage.cr ?? 0 } } });
  m.content.forEach((b, i) => {
    if (b.type === "thinking") {
      push("content_block_start", { index: i, content_block: { type: "thinking", thinking: "", signature: "" } });
      push("content_block_delta", { index: i, delta: { type: "signature_delta", signature: "sig-" + i } });
    } else if (b.type === "text") {
      push("content_block_start", { index: i, content_block: { type: "text", text: "" } });
      for (const c of chunks(b.text, 20)) push("content_block_delta", { index: i, delta: { type: "text_delta", text: c } });
    } else {
      push("content_block_start", { index: i, content_block: { type: "tool_use", id: b.id, name: b.name, input: {} } });
      for (const c of chunks(b.raw ?? JSON.stringify(b.input), 6)) push("content_block_delta", { index: i, delta: { type: "input_json_delta", partial_json: c } });
    }
    push("content_block_stop", { index: i });
  });
  if (m.garbage) ev.push("event: content_block_delta\ndata: {oops\n\n");   // битое событие: SDK бросит не APIError
  push("message_delta", { delta: { stop_reason: m.stop, stop_sequence: null }, usage: { output_tokens: m.usage.output } });
  push("message_stop", {});
  return ev.join("");
}

const FINAL = [
  "## Итог",
  "Похожая проблема была в [#690001](https://hd.example/wrong) и сейчас открыта [#702180](http://127.0.0.1:47899/Task/View/702180).",
  "",
  "| Заявка | Статус |",
  "|---|---|",
  "| #702180 | **Открыта** |",
  "",
  "- почистить `ролик`",
  "- заменить *картридж*",
  "",
  "Не кликать: [сюда](https://evil.example/?q=secret) <img src=x onerror=\"window.pwned=1\">",
].join("\n");

const script = [
  { stop: "tool_use", usage: { input: 1800, output: 60 }, content: [
    { type: "thinking" },
    { type: "text", text: "Поищу похожие заявки." },
    { type: "tool_use", id: "tu_1", name: "search_tickets", input: { query: "принтер бухгалтерия" } },
  ] },
  { stop: "tool_use", usage: { input: 300, output: 80, cw: 2500 }, content: [
    { type: "tool_use", id: "tu_2", name: "get_ticket", input: { id: 702180 } },
    { type: "tool_use", id: "tu_3", name: "get_ticket_history", input: { id: 702180 } },
  ] },
  { stop: "end_turn", usage: { input: 200, output: 400, cr: 4000 }, content: [{ type: "text", text: FINAL }] },
  // второй разговор: вход инструмента не разобрался — SDK отдаёт input {}, страница отвечает ошибкой, Claude решает сам
  { stop: "tool_use", usage: { input: 1000, output: 40 }, content: [
    { type: "text", text: "Сейчас поищу." },
    { type: "tool_use", id: "tu_bad", name: "search_tickets", raw: '{"query": "прин' },
  ] },
  { stop: "end_turn", usage: { input: 1000, output: 50 }, content: [{ type: "text", text: "Ответ после ошибки входа." }] },
  // третий разговор: поток оборвался битым событием — показанное убрано, тот же запрос повторён, попытка в счёте
  { stop: "end_turn", usage: { input: 1000, output: 40 }, garbage: true, content: [{ type: "text", text: "Черновик" }] },
  { stop: "end_turn", usage: { input: 1000, output: 50 }, content: [{ type: "text", text: "Ответ после повтора." }] },
  // третий разговор: ключ не подошёл
  { status: 401, error: { type: "error", error: { type: "authentication_error", message: "invalid x-api-key" } } },
];

(async () => {
  const browser = await chromium.launch();
  const scheme = process.argv[3] ?? "light";
  const page = await browser.newPage({ colorScheme: scheme, viewport: { width: 1100, height: 900 } });
  const problems = [];
  page.on("pageerror", (e) => problems.push(`pageerror: ${e.message}`));
  // ожидаемые: 401 от подменённого API и лог SDK о битом событии в сценарии повтора
  const expected = /status of 401|Could not parse message into JSON|From chunk: \[event: content_block_delta, data: \{oops\]/;
  page.on("console", (m) => { if (m.type() === "error" && !expected.test(m.text())) problems.push(`console: ${m.text()}`); });

  const requests = [];
  let call = 0;
  await page.route("https://api.anthropic.com/**", async (route) => {
    const req = route.request();
    const cors = { "access-control-allow-origin": "*", "access-control-allow-headers": "*", "access-control-allow-methods": "POST, OPTIONS" };
    if (req.method() === "OPTIONS") return route.fulfill({ status: 204, headers: cors });
    requests.push({ url: req.url(), headers: req.headers(), body: JSON.parse(req.postData()) });
    const reply = script[call++];
    if (reply.status) return route.fulfill({ status: reply.status, headers: { ...cors, "content-type": "application/json" }, body: JSON.stringify(reply.error) });
    return route.fulfill({ status: 200, headers: { ...cors, "content-type": "text/event-stream" }, body: sse(reply) });
  });

  // 1. ключ моста из фрагмента: забран в localStorage и стёрт из адреса; без ключа API — открыта настройка
  await page.goto(`${BASE}/#${KEY}`);
  await page.waitForFunction(() => document.getElementById("status").textContent.includes("TicketBoard 0.0.0-check"));
  assert.equal(await page.evaluate(() => location.hash), "");
  assert.equal(await page.evaluate(() => localStorage.getItem("ticketboard.bridgeKey")), KEY);
  assert.equal(await page.isVisible("#setup"), true, "без ключа API видна настройка");
  await page.fill("#apiKey", "sk-ant-test-1234");
  await page.click("#keyForm button[type=submit]");
  assert.equal(await page.isVisible("#setup"), false);

  // 2. вопрос → поиск → заявка и переписка параллельно → ответ
  await page.fill("#input", "Были ли проблемы с принтером в бухгалтерии?");
  await page.press("#input", "Enter");
  await page.waitForSelector(".msg.assistant .meta");

  assert.equal(requests.length, 3);
  const [r1, r2, r3] = requests;
  assert.equal(r1.headers["anthropic-dangerous-direct-browser-access"], "true");
  assert.equal(r1.headers["x-api-key"], "sk-ant-test-1234");
  assert.match(r1.headers["anthropic-beta"], /server-side-fallback-2026-07-01/);
  assert.equal(r1.body.model, "claude-opus-5");
  assert.equal(r1.body.fallbacks, "default");
  assert.deepEqual(r1.body.thinking, { type: "adaptive" });
  assert.deepEqual(r1.body.cache_control, { type: "ephemeral" });
  assert.equal(r1.body.stream, true);
  assert.equal(r1.body.max_tokens, 64000);
  assert.deepEqual(r1.body.tools.map((t) => t.name), ["search_tickets", "get_ticket", "get_ticket_history", "list_board"]);
  assert.ok(r1.body.tools.every((t) => t.eager_input_streaming === true));
  assert.equal(r1.body.betas, undefined, "betas уходят заголовком, не в теле");

  const m2 = r2.body.messages;
  assert.equal(m2.length, 3);
  assert.equal(m2[1].role, "assistant");
  assert.deepEqual(m2[1].content.map((b) => b.type), ["thinking", "text", "tool_use"]);
  assert.equal(m2[1].content[0].signature, "sig-0", "thinking с подписью уходит обратно как есть");
  assert.deepEqual(m2[1].content[2].input, { query: "принтер бухгалтерия" });
  const res1 = m2[2].content[0];
  assert.equal(res1.type, "tool_result");
  assert.equal(res1.tool_use_id, "tu_1");
  assert.equal(res1.is_error, undefined);
  const found = JSON.parse(res1.content);
  assert.equal(found.total, 2);
  assert.equal(found.tickets[0].url, "http://127.0.0.1:47899/Task/View/702180");
  assert.equal(found.tickets[0].description, "HP LaserJet 400 пишет «Замятие бумаги»");

  const last = r3.body.messages.at(-1);
  assert.equal(last.role, "user");
  assert.deepEqual(last.content.map((b) => b.tool_use_id), ["tu_2", "tu_3"], "два результата одним сообщением");
  const ticket = JSON.parse(last.content[0].content);
  assert.equal(ticket.title, "Принтер в бухгалтерии не печатает");
  assert.equal(ticket.board.notes[0].text, "Звонил Петровой");
  const history = JSON.parse(last.content[1].content);
  assert.equal(history.events.length, 2);
  assert.equal(history.events[0].isPublic, false);

  // вывод: инструменты, markdown, ссылки только на Интрасервис, html не исполняется
  assert.equal(await page.locator(".tool.ok").count(), 3);
  assert.match(await page.locator(".tool").first().innerText(), /Поиск: «принтер бухгалтерия»\s*— найдено 2/);
  const answer = page.locator(".msg.assistant").first();
  assert.equal(await answer.locator("h4").innerText(), "Итог");
  assert.equal(await answer.locator("a").count(), 1, "кликабельна только ссылка на Интрасервис");
  assert.equal(await answer.locator("a").getAttribute("href"), "http://127.0.0.1:47899/Task/View/702180");
  assert.match(await answer.innerText(), /#690001 \(https:\/\/hd\.example\/wrong\)/);
  assert.match(await answer.innerText(), /сюда \(https:\/\/evil\.example\/\?q=secret\)/);
  assert.equal(await answer.locator("table td strong").innerText(), "Открыта");
  assert.equal(await answer.locator("li code").innerText(), "ролик");
  assert.equal(await answer.locator("li em").innerText(), "картридж");
  assert.equal(await answer.locator("img").count(), 0);
  assert.equal(await page.evaluate(() => window.pwned), undefined);
  const meta = await answer.locator(".meta").innerText();
  assert.match(meta, /^≈ \$0\.0\d · на входе 9 тыс\. токенов \(из кэша 4 тыс\.\), ответ 540$/, meta);
  assert.equal(await page.isVisible("#stop"), false);
  assert.equal(await page.isEnabled("#send"), true);

  await page.evaluate(() => document.querySelector(".tool details, details.tool").setAttribute("open", ""));
  const shot = path.join(os.tmpdir(), `ticketboard-page-${scheme}.png`);
  await page.screenshot({ path: shot });

  // 3. Новый разговор; вход инструмента не разобрался — ошибка инструмента уходит Claude, разговор продолжается
  await page.click("#newChat");
  await page.fill("#input", "Проверка входа");
  await page.press("#input", "Enter");
  await page.waitForSelector(".msg.assistant .meta");
  assert.equal(requests.length, 5);
  const badResult = requests[4].body.messages.at(-1).content[0];
  assert.equal(badResult.tool_use_id, "tu_bad");
  assert.equal(badResult.is_error, true);
  assert.equal(await page.locator(".tool.error").count(), 1);
  assert.match(await page.locator(".msg.assistant").first().innerText(), /Ответ после ошибки входа\./);

  // 4. Новый разговор; поток оборвался — повтор того же запроса, первая попытка с экрана убрана, но оплачена
  await page.click("#newChat");
  await page.fill("#input", "Проверка повтора");
  await page.press("#input", "Enter");
  await page.waitForSelector(".msg.assistant .meta");
  assert.equal(requests.length, 7);
  assert.deepEqual(requests[6].body.messages, requests[5].body.messages, "повтор — тот же запрос");
  const retried = await page.locator(".msg.assistant").first().innerText();
  assert.doesNotMatch(retried, /Черновик/);
  assert.match(retried, /Ответ после повтора\./);
  // вход: 1000 + 1000; ответ: 1 (попытка оборвалась до message_delta) + 50
  assert.match(await page.locator(".msg.assistant .meta").innerText(), /на входе 2 тыс\. токенов \(из кэша 0\), ответ 51$/);

  // 5. Новый разговор; ключ не подошёл — ошибка, вопрос вернулся в поле, история пуста
  await page.click("#newChat");
  assert.equal(await page.locator(".msg").count(), 0);
  await page.fill("#input", "Что на доске?");
  await page.click("#send");
  await page.waitForSelector(".msg.assistant .error");
  assert.match(await page.locator(".msg.assistant .error").innerText(), /Ключ API не подошёл/);
  assert.equal(await page.inputValue("#input"), "Что на доске?");
  assert.equal(await page.isVisible("#setup"), true);
  assert.equal(requests.at(-1).body.messages.length, 1, "новый разговор начинается с чистой истории");

  // 6. без ключа моста в запросе API не отвечает
  const status = await page.evaluate(async () => (await fetch("/api/board")).status);
  assert.equal(status, 401);

  assert.deepEqual(problems, []);
  await browser.close();
  console.log(`page ${scheme}: OK (снимок ответа — ${shot})`);
})().catch((e) => { console.error(e); process.exit(1); });
