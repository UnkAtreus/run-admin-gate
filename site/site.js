"use strict";

/* ---------- Copy buttons on terminal windows ---------- */
document.querySelectorAll(".term .copy").forEach((button) => {
  button.addEventListener("click", () => {
    const pre = button.closest(".term").querySelector("pre");
    const clone = pre.cloneNode(true);
    clone.querySelectorAll(".p, .c").forEach((el) => el.remove());
    const text = clone.textContent.split("\n").map((l) => l.trimEnd()).filter(Boolean).join("\n");
    const done = (label) => {
      button.textContent = label;
      setTimeout(() => { button.textContent = "Copy"; }, 1600);
    };
    const select = () => {
      const range = document.createRange();
      range.selectNodeContents(pre);
      const sel = window.getSelection();
      sel.removeAllRanges();
      sel.addRange(range);
      done("Press Ctrl+C");
    };
    try {
      navigator.clipboard.writeText(text).then(() => done("Copied"), select);
    } catch (e) {
      select();
    }
  });
});

/* ---------- Guide: highlight the step being read ---------- */
const tocLinks = [...document.querySelectorAll(".toc a[href^='#']")];
if (tocLinks.length && "IntersectionObserver" in window) {
  const byId = new Map(tocLinks.map((a) => [a.getAttribute("href").slice(1), a]));
  const visible = new Set();
  const observer = new IntersectionObserver((entries) => {
    entries.forEach((e) => (e.isIntersecting ? visible.add(e.target.id) : visible.delete(e.target.id)));
    const first = [...byId.keys()].find((id) => visible.has(id));
    if (first) tocLinks.forEach((a) => a.classList.toggle("on", a === byId.get(first)));
  }, { rootMargin: "-80px 0px -55% 0px" });
  byId.forEach((_, id) => { const el = document.getElementById(id); if (el) observer.observe(el); });
}

/* ---------- Rule tester: same rules as the service's allow-list matcher ---------- */
(function ruleTester() {
  const out = document.getElementById("t-out");
  if (!out) return;
  const ruleIn = document.getElementById("t-rule");
  const programIn = document.getElementById("t-program");
  const argsIn = document.getElementById("t-args");

  // The service expands %VAR% with its own (SYSTEM) environment.
  const ENV = {
    windir: "C:\\Windows", systemroot: "C:\\Windows", systemdrive: "C:",
    programfiles: "C:\\Program Files", "programfiles(x86)": "C:\\Program Files (x86)",
    programdata: "C:\\ProgramData", commonprogramfiles: "C:\\Program Files\\Common Files",
  };
  const LAUNCHABLE = [".exe", ".com", ".cmd", ".bat", ".msi", ".msp", ".vbs", ".ps1"];

  const expand = (s) => s.replace(/%([^%]+)%/g, (m, k) => ENV[k.toLowerCase()] ?? m);
  // One-to-one uppercase per character, like the service.
  const fold = (s) => [...s].map((c) => { const u = c.toUpperCase(); return u.length === 1 ? u : c; }).join("");

  function normalize(p) {
    p = p.replace(/\//g, "\\");
    if (p.startsWith("\\\\?\\UNC\\")) p = "\\\\" + p.slice(8);
    else if (p.startsWith("\\\\?\\")) p = p.slice(4);
    let root, rest;
    if (p.startsWith("\\\\")) {
      const unc = p.slice(2);
      if (/^[.?]/.test(unc)) return null;
      const parts = unc.split("\\");
      if (!parts[0] || !parts[1]) return null;
      root = "\\\\" + parts[0] + "\\" + parts[1];
      rest = parts.slice(2).join("\\");
    } else {
      if (!/^[A-Za-z]:\\/.test(p)) return null;
      root = p[0].toUpperCase() + ":";
      rest = p.slice(3);
    }
    const comps = [];
    for (const raw of rest.split("\\")) {
      if (raw === "" || raw === ".") continue;
      if (raw === "..") { comps.pop(); continue; }
      const t = raw.replace(/[. ]+$/, "");
      if (t) comps.push(t);
    }
    return { root, comps };
  }
  const pathString = (n) => (n.comps.length ? n.root + "\\" + n.comps.join("\\") : n.root + "\\");

  function pathTokens(pattern) {
    const out = [];
    const ch = [...pattern];
    for (let i = 0; i < ch.length; i++) {
      if (ch[i] === "*" && ch[i + 1] === "*") {
        while (ch[i + 1] === "*") i++;
        out.push({ t: "star" });
      } else if (ch[i] === "*") out.push({ t: "starName" });
      else if (ch[i] === "?") out.push({ t: "oneName" });
      else out.push({ t: "lit", c: ch[i] });
    }
    return out;
  }
  const argTokens = (pattern) => [...pattern].map((c) => (c === "*" ? { t: "star" } : c === "?" ? { t: "one" } : { t: "lit", c }));

  function matches(tokens, text) {
    const n = text.length;
    let cur = new Array(n + 1).fill(false);
    cur[0] = true;
    for (const tok of tokens) {
      const next = new Array(n + 1).fill(false);
      if (tok.t === "lit") for (let i = 0; i < n; i++) next[i + 1] = cur[i] && text[i] === tok.c;
      else if (tok.t === "one") for (let i = 0; i < n; i++) next[i + 1] = cur[i];
      else if (tok.t === "oneName") for (let i = 0; i < n; i++) next[i + 1] = cur[i] && text[i] !== "\\";
      else {
        let active = false;
        for (let i = 0; i <= n; i++) {
          active = active || cur[i];
          next[i] = active;
          if (i < n && tok.t === "starName" && text[i] === "\\") active = false;
        }
      }
      if (!next.some(Boolean)) return false;
      cur = next;
    }
    return cur[n];
  }

  function programEnd(text) {
    const lower = text.toLowerCase();
    let best = null;
    for (const ext of LAUNCHABLE) {
      let from = 0;
      for (;;) {
        const pos = lower.indexOf(ext, from);
        if (pos < 0) break;
        const end = pos + ext.length;
        if (end === lower.length || /\s/.test(lower[end])) { best = best === null ? end : Math.min(best, end); break; }
        from = end;
      }
    }
    return best;
  }

  function parseEntry(entry) {
    const mode = entry.includes("/asadmin") ? "admin" : entry.includes("/assystem") ? "system" : null;
    const base = entry.split("/assystem").join("").split("/asadmin").join("").trim();
    if (base.startsWith('"')) {
      const q = base.slice(1);
      const end = q.indexOf('"');
      const path = end < 0 ? q : q.slice(0, end);
      return { path: path.trim(), args: end < 0 ? "" : q.slice(end + 1).trim(), mode, folder: path.trimEnd().endsWith("\\") };
    }
    if (base.endsWith("\\")) return { path: base, args: "", mode, folder: true };
    const end = programEnd(base) ?? base.length;
    return { path: base.slice(0, end), args: base.slice(end).trim(), mode, folder: false };
  }

  function compile(entry) {
    const e = parseEntry(entry);
    const n = normalize(expand(e.path));
    if (!n) return { error: "not an absolute path (use C:\\... or \\\\server\\share\\...)" };
    if (/[*?]/.test(n.root)) return { error: "wildcards are not allowed in the drive or share name" };
    let pattern = pathString(n);
    if (e.folder) { if (n.comps.length) pattern += "\\"; pattern += "*"; }
    else if (!n.comps.length) return { error: "a drive or share root is not a program; end folders with \\" };
    const bare = e.args.replace(/^"+|"+$/g, "");
    const args = bare === "" ? { kind: "none" } : bare === "%1" ? { kind: "file" } : { kind: "pattern", toks: argTokens(fold(e.args)) };
    // For folder entries, the same folder with subfolders, to explain a near miss.
    const deep = e.folder ? pathTokens(fold(pattern + "*")) : null;
    return { mode: e.mode, deep, path: pathTokens(fold(pattern)), args };
  }

  function splitArg(s) {
    s = s.replace(/^\s+/, "");
    if (!s) return null;
    if (s[0] === '"') {
      const end = s.indexOf('"', 1);
      return end < 0 ? [s.slice(1), ""] : [s.slice(1, end), s.slice(end + 1)];
    }
    const m = s.match(/^(\S+)([\s\S]*)$/);
    return [m[1], m[2]];
  }

  function acceptArgs(rule, args) {
    if (rule.kind === "none") return args === "" ? "" : null;
    if (rule.kind === "pattern") return matches(rule.toks, [...fold(args)]) ? args : null;
    if (args === "") return "";
    const split = splitArg(args);
    if (!split || split[1].trim() !== "") return null;
    const n = normalize(split[0]);
    if (!n || !n.comps.length) return null;
    return '"' + pathString(n) + '"';
  }

  const ICONS = {
    allow: '<circle cx="12" cy="12" r="9"/><path d="m8 12.5 2.6 2.5L16 9.5"/>',
    deny: '<circle cx="12" cy="12" r="9"/><path d="m9 9 6 6m0-6-6 6"/>',
    skip: '<path d="M12 4 2.8 19.5h18.4z"/><path d="M12 10v4m0 2.6v.1"/>',
  };
  function show(kind, title, detail) {
    out.className = "verdict " + kind;
    out.innerHTML = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">' +
      ICONS[kind] + "</svg><div><strong></strong><span></span></div>";
    out.querySelector("strong").textContent = title;
    out.querySelector("span").textContent = detail;
  }

  function run() {
    const entry = ruleIn.value.trim();
    const program = programIn.value.trim();
    const args = argsIn.value.trim();
    if (!entry) return show("skip", "No entry", "Type an AllowedPath entry, for example C:\\Tools\\app.exe.");
    const rule = compile(entry);
    if (rule.error) return show("skip", "Entry ignored (Event ID 20)", "The service skips this entry: " + rule.error + ".");
    const n = normalize(program);
    if (!n) return show("deny", "Refused (Event ID 2)", "The program path must be a full path.");
    const text = [...fold(pathString(n))];
    if (!matches(rule.path, text)) {
      const hint = /\\\.\.?(\\|$)/.test(program.replace(/\//g, "\\")) ? " The path was normalized to " + pathString(n) + " first."
        : rule.deep && matches(rule.deep, text) ? " Folder entries don't include subfolders; end the entry with \\** for that." : "";
      return show("deny", "Refused (Event ID 2)", "This program is not on the RunAdminGate allow-list." + hint);
    }
    const passed = acceptArgs(rule.args, args);
    if (passed === null) {
      const want = rule.args.kind === "none" ? "This entry allows no arguments." : rule.args.kind === "file" ? "This entry allows nothing or one full file path." : "The arguments must match the entry's arguments as a whole.";
      return show("deny", "Refused (Event ID 2)", "The program matches, but the arguments don't. " + want);
    }
    const who = rule.mode === "admin" ? "Starts as the user with administrator rights: " : "Starts as SYSTEM: ";
    show("allow", "Allowed (Event ID 1)", who + "\"" + pathString(n) + "\"" + (passed ? " " + passed : "") + ".");
  }

  const chips = [...document.querySelectorAll(".presets .chip")];
  chips.forEach((chip) => chip.addEventListener("click", () => {
    ruleIn.value = chip.dataset.rule;
    programIn.value = chip.dataset.program;
    argsIn.value = chip.dataset.args;
    chips.forEach((c) => c.setAttribute("aria-pressed", String(c === chip)));
    run();
  }));
  [ruleIn, programIn, argsIn].forEach((el) => el.addEventListener("input", () => {
    chips.forEach((c) => c.setAttribute("aria-pressed", "false"));
    run();
  }));
  chips.forEach((c, i) => c.setAttribute("aria-pressed", String(i === 0)));
  run();
})();
