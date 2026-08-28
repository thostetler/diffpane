const $ = (tag, props = {}, children = []) => {
  const el = document.createElement(tag);
  for (const [key, value] of Object.entries(props)) {
    if (value === undefined || value === null) continue;
    if (key === "class") el.className = value;
    else if (key === "text") el.textContent = value;
    else if (key === "dataset") Object.assign(el.dataset, value);
    else if (key.startsWith("on")) el.addEventListener(key.slice(2).toLowerCase(), value);
    else if (key === "ariaExpanded") el.setAttribute("aria-expanded", String(value));
    else el.setAttribute(key, value);
  }
  for (const child of Array.isArray(children) ? children : [children]) {
    if (child !== undefined && child !== null) el.append(child);
  }
  return el;
};

const fixtureMode = new URLSearchParams(location.search).get("fixture") === "1";
const token = new URLSearchParams(location.search).get("t") || "";

const state = {
  data: null,
  filesById: new Map(),
  filesByPath: new Map(),
  hunksById: new Map(),
  hunkFile: new Map(),
  chapters: [],
  comments: [],
  collapsedFiles: new Set(),
  expandedHunks: new Set(),
  focusedLine: null,
  focusedAnchor: null,
  focusedHunk: null,
  focusedChapter: null,
  composer: null,
  offline: false,
  actionError: null,
  busy: false
};

async function boot() {
  try {
    const payload = await loadInitial();
    setData(payload);
    render();
  } catch (error) {
    document.getElementById("app").replaceChildren(
      $("main", { class: "main" }, [
        $("div", { class: "top" }, [
          $("h1", { text: "diffpane" }),
          $("p", { class: "error", text: error.message || "Unable to load review." })
        ])
      ])
    );
  }
}

async function loadInitial() {
  if (fixtureMode) {
    return fetchJson("/assets/fixture.json");
  }
  return fetchJson("/api/review");
}

// The API is gated on a custom header: cross-origin callers cannot set one
// without a preflight, which is what keeps other browser tabs out.
function authorize(path, options) {
  if (path.startsWith("/api/")) {
    return [path, { ...options, headers: { ...(options?.headers || {}), "X-Diffpane-Token": token } }];
  }
  return [token ? `${path}${path.includes("?") ? "&" : "?"}t=${encodeURIComponent(token)}` : path, options];
}

async function fetchJson(path, options) {
  const response = await fetch(...authorize(path, options));
  if (!response.ok) {
    let message = `${response.status} ${response.statusText}`;
    try {
      const body = await response.json();
      if (body.error) message = body.error;
    } catch {}
    const error = new Error(message);
    // Tagged so mutate() can tell a server rejection from a dead connection.
    error.status = response.status;
    throw error;
  }
  return response.json();
}

function setData(payload) {
  payload.review ||= { title: "Review unavailable", story: "review.json is missing.", chapters: [], file_notes: {} };
  payload.comments ||= { comments: [], progress: {}, overall: { verdict: "ok", body: "" }, submitted: false, submitted_at: null };
  payload.comments.comments ||= [];
  payload.comments.progress ||= {};
  payload.comments.overall ||= { verdict: "ok", body: "" };
  state.data = payload;
  state.comments = payload.comments.comments;
  indexData();
  seedFolds();
}

function indexData() {
  state.filesById.clear();
  state.filesByPath.clear();
  state.hunksById.clear();
  state.hunkFile.clear();
  const claimed = new Set();
  for (const file of state.data.hunks?.files || []) {
    state.filesById.set(file.id, file);
    state.filesByPath.set(file.path, file);
    for (const hunk of file.hunks || []) {
      state.hunksById.set(hunk.id, hunk);
      state.hunkFile.set(hunk.id, file);
    }
  }
  const chapters = (state.data.review?.chapters || []).map(chapter => {
    for (const id of chapter.hunks || []) claimed.add(id);
    return { ...chapter, synthetic: false };
  });
  const unclaimed = [];
  const zeroHunkFiles = [];
  for (const id of state.hunksById.keys()) {
    if (!claimed.has(id)) unclaimed.push(id);
  }
  for (const file of state.data.hunks?.files || []) {
    if (!file.hunks?.length) zeroHunkFiles.push(file.path);
  }
  chapters.push({ id: "unsorted", title: "Everything else", intent: "Hunks not claimed by review.json.", why: "", hunks: unclaimed, files: zeroHunkFiles, size: "", flags: [], synthetic: true });
  state.chapters = chapters;
}

function seedFolds() {
  if (state.collapsedFiles.size) return;
  const commentedFiles = new Set(state.comments.map(comment => comment.anchor?.file).filter(Boolean));
  for (const file of state.data.hunks?.files || []) {
    const lineCount = (file.hunks || []).reduce((sum, hunk) => sum + (hunk.lines?.length || 0), 0);
    if (!commentedFiles.has(file.path) && (file.noise || file.status === "added" || file.status === "deleted" || lineCount > 400)) {
      state.collapsedFiles.add(file.path);
    }
  }
}

function render() {
  const app = document.getElementById("app");
  const sidebar = renderSidebar();
  const main = $("main", { class: "main" }, [renderHeader(), ...state.chapters.map(renderChapter), renderFooter()]);
  app.replaceChildren(sidebar, main, renderHelp());
  markActiveNav();
}

function renderSidebar() {
  return $("aside", { class: "sidebar" }, [
    $("div", { class: "brand" }, [
      $("div", { class: "brand-title", text: "diffpane" }),
      $("button", { type: "button", text: "?", title: "Shortcuts", onClick: showHelp })
    ]),
    $("div", {
      class: "banner" + (state.offline || state.actionError ? " show" : ""),
      text: state.actionError
        || "server unreachable - your last change was not saved"
    }),
    $("nav", { class: "nav-list", "aria-label": "Chapters" },
      state.chapters.map(chapter => $("button", {
        type: "button",
        class: "nav-item",
        dataset: { chapter: chapter.id },
        onClick: () => scrollToChapter(chapter.id)
      }, [
        $("span", { class: "nav-title", text: chapter.title }),
        $("span", { class: "dot " + (chapterState(chapter.id) === "reviewed" ? "reviewed" : "") })
      ]))
    )
  ]);
}

function renderHeader() {
  const { meta, review } = state.data;
  const reviewed = state.chapters.filter(chapter => chapterState(chapter.id) === "reviewed").length;
  const total = state.chapters.length;
  const range = meta?.base && meta?.head ? `${meta.base}...${meta.head}` : meta?.diff_cmd || "";
  const totals = meta?.totals ? `${meta.totals.files} files  +${meta.totals.additions}/-${meta.totals.deletions}` : "no totals";
  return $("header", { class: "top" }, [
    $("div", { class: "top-row" }, [
      $("div", {}, [
        $("h1", { text: review?.title || "Review" }),
        $("p", { class: "story", text: review?.story || "No review narrative found." }),
        $("p", { class: "meta-line", text: `${meta?.repo || "repo"} / ${meta?.scope || "scope"} / ${range} / ${totals}` })
      ]),
      $("div", { class: "toolbar" }, [
        $("span", { class: "badge", text: `${reviewed}/${total} chapters reviewed` }),
        $("button", { type: "button", text: "expand all", onClick: () => setAllExpanded(true) }),
        $("button", { type: "button", text: "collapse all", onClick: () => setAllExpanded(false) })
      ])
    ])
  ]);
}

function renderChapter(chapter) {
  const section = $("section", { class: "chapter", id: `chapter-${chapter.id}`, dataset: { chapter: chapter.id } });
  const badges = $("div", { class: "badges" }, [
    chapter.size ? $("span", { class: "badge", text: chapter.size }) : null,
    $("span", { class: "badge", text: `${(chapter.hunks || []).length} hunks` })
  ].filter(Boolean));
  section.append($("div", { class: "section-head" }, [
    $("div", {}, [$("h2", { text: chapter.title }), badges]),
    $("div", { class: "chapter-actions" }, [
      $("button", { type: "button", text: "comment", onClick: () => openComposer({ kind: "chapter", chapter: chapter.id }, null) }),
      $("button", {
        type: "button",
        text: chapterState(chapter.id) === "reviewed" ? "reviewed" : "mark reviewed",
        onClick: guard(() => toggleChapter(chapter.id))
      })
    ])
  ]));
  section.append($("div", { class: "chapter-copy" }, [
    $("p", {}, [$("strong", { text: "Intent " }), document.createTextNode(chapter.intent || "")]),
    $("p", {}, [$("strong", { text: "Why " }), document.createTextNode(chapter.why || "")])
  ]));
  if (chapter.flags?.length) {
    section.append($("div", { class: "flags" }, chapter.flags.map(flag => $("span", { class: "badge warn", text: flag }))));
  }
  appendAnchorComments(section, { kind: "chapter", chapter: chapter.id });
  const groups = groupChapterHunks(chapter);
  if (!groups.length) section.append($("div", { class: "empty", text: "No hunks." }));
  for (const group of groups) section.append(renderFileGroup(chapter, group));
  return section;
}

function groupChapterHunks(chapter) {
  const groups = [];
  const byPath = new Map();
  for (const hunkId of chapter.hunks || []) {
    const hunk = state.hunksById.get(hunkId);
    const file = state.hunkFile.get(hunkId);
    if (!hunk || !file) {
      groups.push({ missing: hunkId });
      continue;
    }
    if (!byPath.has(file.path)) {
      const group = { file, hunks: [] };
      byPath.set(file.path, group);
      groups.push(group);
    }
    byPath.get(file.path).hunks.push(hunk);
  }
  for (const path of chapter.files || []) {
    const file = state.filesByPath.get(path);
    if (file && !byPath.has(file.path)) groups.push({ file, hunks: [] });
  }
  return groups;
}

function renderFileGroup(chapter, group) {
  if (group.missing) return $("div", { class: "missing", text: `Unknown hunk referenced: ${group.missing}` });
  const { file, hunks } = group;
  const collapsed = state.collapsedFiles.has(file.path);
  const lineCount = hunks.reduce((sum, hunk) => sum + (hunk.lines?.length || 0), 0);
  const article = $("article", { class: "file" + (file.noise ? " noise" : ""), dataset: { file: file.path } });
  article.append($("div", { class: "file-head" }, [
    $("div", { class: "path", title: file.path, text: file.path }),
    $("div", { class: "file-actions" }, [
      $("span", { class: "badge", text: file.status }),
      $("span", { class: "badge", text: `+${file.additions}/-${file.deletions}` }),
      file.noise ? $("span", { class: "badge warn", text: "noise" }) : null,
      file.binary ? $("span", { class: "badge warn", text: "binary" }) : null,
      $("button", { type: "button", text: "comment", onClick: () => openComposer({ kind: "file", file: file.path }, null) }),
      $("button", {
        type: "button",
        text: collapsed ? "expand" : "collapse",
        ariaExpanded: !collapsed,
        onClick: () => toggleFile(file.path)
      })
    ].filter(Boolean))
  ]));
  const note = state.data.review?.file_notes?.[file.path];
  if (note) article.append($("div", { class: "file-note", text: note }));
  appendAnchorComments(article, { kind: "file", file: file.path });
  if (file.binary) {
    article.append($("div", { class: "collapsed-summary", text: "Binary file changed; diff body unavailable." }));
    return article;
  }
  if (file.truncated) article.append($("div", { class: "truncated", text: "diff truncated" }));
  if (collapsed) {
    article.append($("div", { class: "collapsed-summary", text: `${hunks.length} hunks, ${lineCount} lines hidden.` }));
    return article;
  }
  if (!hunks.length) {
    article.append($("div", { class: "empty", text: "No hunks in this file." }));
    return article;
  }
  const diff = $("div", { class: "diff" });
  for (const hunk of hunks) diff.append(renderHunk(chapter, file, hunk));
  article.append(diff);
  return article;
}

function renderHunk(chapter, file, hunk) {
  const wrap = $("div", { class: "hunk", id: `hunk-${hunk.id}`, dataset: { hunk: hunk.id, chapter: chapter.id, file: file.path } });
  wrap.append($("div", { class: "hunk-header", text: hunk.header || hunk.id }));
  const lines = hunk.lines || [];
  const hasComment = lines.some(line => commentsForAnchor(lineAnchor(file, hunk, line, chapter.id)).length);
  const shouldFold = lines.length > 40 && !state.expandedHunks.has(hunk.id) && !hasComment;
  const visible = shouldFold ? [...lines.slice(0, 8), ...lines.slice(-8)] : lines;
  for (let idx = 0; idx < visible.length; idx++) {
    if (shouldFold && idx === 8) {
      wrap.append($("div", { class: "fold-row" }, [
        $("button", { type: "button", text: `${lines.length - 16} lines folded`, onClick: () => { state.expandedHunks.add(hunk.id); render(); scrollToHunk(hunk.id); } })
      ]));
    }
    appendLine(wrap, chapter, file, hunk, visible[idx]);
  }
  return wrap;
}

function appendLine(parent, chapter, file, hunk, line) {
  const anchor = lineAnchor(file, hunk, line, chapter.id);
  const key = anchorKey(anchor);
  const row = $("div", {
    class: `diff-row ${line.type}`,
    id: `line-${hunk.id}-${line.i}`,
    tabindex: "0",
    dataset: { lineKey: key, hunk: hunk.id, chapter: chapter.id, file: file.path },
    onFocus: () => setFocused(key, anchor, hunk.id, chapter.id),
    onClick: () => setFocused(key, anchor, hunk.id, chapter.id)
  }, [
    $("button", { type: "button", class: "gutter", text: line.old ?? "", onClick: event => { event.stopPropagation(); openComposer(anchor, key); } }),
    $("button", { type: "button", class: "gutter", text: line.new ?? "", onClick: event => { event.stopPropagation(); openComposer(anchor, key); } }),
    $("button", { type: "button", class: "add-comment", text: "+", title: "Comment", onClick: event => { event.stopPropagation(); openComposer(anchor, key); } }),
    $("div", { class: "code", text: line.text ?? "" })
  ]);
  parent.append(row);
  if (state.composer?.key === key) parent.append(renderComposer());
  for (const comment of commentsForAnchor(anchor)) parent.append(renderComment(comment));
}

function lineAnchor(file, hunk, line, chapter) {
  const side = line.type === "del" ? "old" : "new";
  return { kind: "line", file: file.path, hunk: hunk.id, side, line: side === "old" ? line.old : line.new, chapter };
}

function renderComment(comment) {
  const box = $("div", { class: "comment-box" });
  box.append($("div", { class: "comment-meta" }, [
    $("span", {}, [
      $("span", { class: `verdict ${comment.verdict}`, text: comment.verdict }),
      document.createTextNode(comment.resolved ? " / resolved" : " / open")
    ]),
    $("span", { class: "comment-actions" }, [
      $("button", { type: "button", text: "edit", onClick: () => openComposer(comment.anchor, anchorKey(comment.anchor), comment) }),
      $("button", { type: "button", text: comment.resolved ? "reopen" : "resolve", onClick: guard(() => patchComment(comment.id, { resolved: !comment.resolved })) }),
      $("button", { type: "button", text: "delete", onClick: guard(() => deleteComment(comment.id)) })
    ])
  ]));
  box.append($("div", { class: "comment-body", text: comment.body || "" }));
  return box;
}

function renderComposer(existing = state.composer?.comment) {
  const verdict = existing?.verdict || state.composer?.verdict || "fix";
  const body = existing?.body ?? state.composer?.body ?? "";
  const form = $("form", { class: "composer", onSubmit: event => { event.preventDefault(); saveComposer(form); } });
  form.append($("div", { class: "radios" }, ["ok", "fix", "question"].map(value => {
    const input = $("input", { type: "radio", name: "verdict", value });
    input.checked = value === verdict;
    return $("label", {}, [input, document.createTextNode(" " + value)]);
  })));
  const textarea = $("textarea", { name: "body", placeholder: "Comment" });
  textarea.value = body;
  form.append(textarea);
  form.append($("div", { class: "composer-actions" }, [
    $("button", { type: "submit", text: "Save" }),
    $("button", { type: "button", text: "Cancel", onClick: closeComposer })
  ]));
  if (state.composer?.error) form.append($("div", { class: "error", text: state.composer.error }));
  setTimeout(() => trapComposer(form), 0);
  return form;
}

function appendAnchorComments(parent, anchor) {
  if (state.composer?.key === anchorKey(anchor)) parent.append(renderComposer());
  for (const comment of commentsForAnchor(anchor)) parent.append(renderComment(comment));
}

function openComposer(anchor, key, comment) {
  state.composer = { anchor, key: key || anchorKey(anchor), comment, verdict: comment?.verdict || "fix", body: comment?.body || "" };
  render();
  document.querySelector(".composer textarea")?.focus();
}

function closeComposer() {
  state.composer = null;
  render();
}

async function saveComposer(form) {
  const body = form.elements.body.value;
  const verdict = new FormData(form).get("verdict") || "fix";
  const comment = state.composer.comment;
  state.composer.body = body;
  state.composer.verdict = verdict;
  try {
    if (comment) await patchComment(comment.id, { verdict, body }, false);
    else await createComment({ anchor: state.composer.anchor, verdict, body });
    state.composer = null;
    render();
  } catch (error) {
    state.composer.error = error.message;
    render();
  }
}

function trapComposer(form) {
  const focusables = [...form.querySelectorAll("button, textarea, input")];
  form.addEventListener("keydown", event => {
    if ((event.metaKey || event.ctrlKey) && event.key === "Enter") {
      event.preventDefault();
      saveComposer(form);
    } else if (event.key === "Escape") {
      event.preventDefault();
      closeComposer();
    } else if (event.key === "Tab" && focusables.length) {
      const first = focusables[0];
      const last = focusables[focusables.length - 1];
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    }
  });
}

async function createComment(payload) {
  const created = await mutate("/api/comments", { method: "POST", body: JSON.stringify(payload) }, () => ({
    id: "local-" + crypto.randomUUID(),
    ...payload,
    created_at: new Date().toISOString(),
    updated_at: new Date().toISOString(),
    resolved: false
  }));
  state.comments.push(created);
}

async function patchComment(id, patch, rerender = true) {
  const updated = await mutate(`/api/comments/${encodeURIComponent(id)}`, { method: "PATCH", body: JSON.stringify(patch) }, () => {
    const current = state.comments.find(comment => comment.id === id);
    return { ...current, ...patch, updated_at: new Date().toISOString() };
  });
  const index = state.comments.findIndex(comment => comment.id === id);
  if (index >= 0) state.comments[index] = updated;
  if (rerender) render();
}

async function deleteComment(id) {
  await mutate(`/api/comments/${encodeURIComponent(id)}`, { method: "DELETE" }, () => ({ ok: true }));
  state.comments = state.comments.filter(comment => comment.id !== id);
  state.data.comments.comments = state.comments;
  render();
}

async function toggleChapter(chapter) {
  const stateValue = chapterState(chapter) === "reviewed" ? "unreviewed" : "reviewed";
  const result = await mutate("/api/progress", { method: "PUT", body: JSON.stringify({ chapter, state: stateValue }) }, () => ({ progress: { ...state.data.comments.progress, [chapter]: stateValue } }));
  state.data.comments.progress = result.progress;
  render();
}

async function saveOverall(overall) {
  const result = await mutate("/api/overall", { method: "PUT", body: JSON.stringify(overall) }, () => ({ overall }));
  state.data.comments.overall = result.overall;
}

async function submitReview() {
  const unreviewed = state.chapters.filter(chapter => chapterState(chapter.id) !== "reviewed");
  if (unreviewed.length && !confirm(`${unreviewed.length} chapters are unreviewed. Finish anyway?`)) return;
  const overall = readOverall();
  await saveOverall(overall);
  const result = await mutate("/api/submit", { method: "POST", body: JSON.stringify({ overall }) }, () => ({ submitted: true, submitted_at: new Date().toISOString() }));
  state.data.comments.submitted = result.submitted;
  state.data.comments.submitted_at = result.submitted_at;
  render();
}

// Wraps a fire-and-forget handler so a rejected mutation shows up instead of
// becoming an unhandled promise rejection.
function guard(run) {
  return () => {
    run().catch(error => {
      state.actionError = error.message || "Request failed.";
      render();
    });
  };
}

// Every failure propagates. The server owns the review and exits when it ends,
// so a change it did not accept did not happen; pretending otherwise loses
// comments silently.
async function mutate(path, options, localApply) {
  const headers = { "Content-Type": "application/json" };
  if (fixtureMode) return localApply();
  try {
    const result = await fetchJson(path, { ...options, headers });
    state.offline = false;
    state.actionError = null;
    return result;
  } catch (error) {
    state.offline = error.status === undefined;
    throw error;
  }
}

function renderFooter() {
  const comments = state.data.comments;
  if (comments.submitted) {
    return $("footer", { class: "footer" }, [
      $("div", { class: "footer-inner" }, [
        $("span", { class: "done", text: `Submitted ${comments.submitted_at || ""}` }),
        $("span", { class: "subline", text: "Return to Claude." })
      ])
    ]);
  }
  const overall = comments.overall || { verdict: "ok", body: "" };
  const form = $("footer", { class: "footer" });
  const verdictSelect = $("select", { name: "overall-verdict", "aria-label": "Overall verdict" },
    ["ok", "fix", "question"].map(value => {
      const option = $("option", { value, text: value });
      option.selected = value === overall.verdict;
      return option;
    })
  );
  const notes = $("textarea", { class: "overall-notes", name: "overall-body", placeholder: "Overall notes" });
  notes.value = overall.body || "";
  notes.addEventListener("change", () => saveOverall(readOverall()));
  verdictSelect.addEventListener("change", () => saveOverall(readOverall()));
  form.append($("div", { class: "overall" }, [
    $("div", {}, [
      verdictSelect,
      $("div", { class: "subline", text: `${state.comments.filter(comment => !comment.resolved).length} open comments` })
    ]),
    notes,
    $("button", { type: "button", text: "Finish review", onClick: guard(submitReview) })
  ]));
  return form;
}

function readOverall() {
  return {
    verdict: document.querySelector("[name='overall-verdict']")?.value || state.data.comments.overall?.verdict || "ok",
    body: document.querySelector("[name='overall-body']")?.value || ""
  };
}

function commentsForAnchor(anchor) {
  const key = anchorKey(anchor);
  return state.comments.filter(comment => anchorKey(comment.anchor) === key);
}

function anchorKey(anchor) {
  if (!anchor) return "";
  if (anchor.kind === "line") return `line:${anchor.file}:${anchor.hunk}:${anchor.side}:${anchor.line}`;
  if (anchor.kind === "file") return `file:${anchor.file}`;
  if (anchor.kind === "chapter") return `chapter:${anchor.chapter}`;
  return "overall";
}

function chapterState(chapter) {
  return state.data.comments.progress?.[chapter] || "unreviewed";
}

function toggleFile(path) {
  if (state.collapsedFiles.has(path)) state.collapsedFiles.delete(path);
  else state.collapsedFiles.add(path);
  render();
}

function setAllExpanded(expanded) {
  state.collapsedFiles.clear();
  if (!expanded) {
    for (const file of state.data.hunks?.files || []) state.collapsedFiles.add(file.path);
  }
  if (expanded) {
    for (const id of state.hunksById.keys()) state.expandedHunks.add(id);
  } else {
    state.expandedHunks.clear();
  }
  render();
}

function setFocused(key, anchor, hunk, chapter) {
  state.focusedLine = key;
  state.focusedAnchor = anchor;
  state.focusedHunk = hunk;
  state.focusedChapter = chapter;
  markActiveNav();
}

function scrollToChapter(id) {
  document.getElementById(`chapter-${id}`)?.scrollIntoView({ block: "start" });
  state.focusedChapter = id;
  markActiveNav();
}

function scrollToHunk(id) {
  const file = state.hunkFile.get(id);
  if (file) state.collapsedFiles.delete(file.path);
  render();
  const el = document.getElementById(`hunk-${id}`);
  el?.scrollIntoView({ block: "center" });
  el?.querySelector(".diff-row")?.focus();
}

function markActiveNav() {
  document.querySelectorAll(".nav-item").forEach(item => {
    item.classList.toggle("active", item.dataset.chapter === state.focusedChapter);
  });
}

function renderHelp() {
  const overlay = $("div", { class: "overlay", id: "help-overlay", onClick: event => { if (event.target.id === "help-overlay") hideHelp(); } });
  overlay.append($("div", { class: "help", role: "dialog", "aria-modal": "true", "aria-label": "Keyboard shortcuts" }, [
    $("h2", { text: "Shortcuts" }),
    $("dl", {}, [
      $("dt", { text: "j / k" }), $("dd", { text: "next / previous hunk" }),
      $("dt", { text: "n / p" }), $("dd", { text: "next / previous chapter" }),
      $("dt", { text: "c" }), $("dd", { text: "comment focused line" }),
      $("dt", { text: "e" }), $("dd", { text: "expand / collapse focused file" }),
      $("dt", { text: "?" }), $("dd", { text: "toggle help" }),
      $("dt", { text: "Esc" }), $("dd", { text: "close" })
    ]),
    $("p", { class: "subline", text: "Cmd/Ctrl+Enter saves a comment." })
  ]));
  return overlay;
}

function showHelp() {
  document.getElementById("help-overlay")?.classList.add("show");
}

function hideHelp() {
  document.getElementById("help-overlay")?.classList.remove("show");
}

document.addEventListener("keydown", event => {
  if (event.target.closest?.(".composer, textarea, input, select")) return;
  if (event.key === "?") {
    event.preventDefault();
    const overlay = document.getElementById("help-overlay");
    overlay?.classList.toggle("show");
  } else if (event.key === "Escape") {
    hideHelp();
  } else if (event.key === "j" || event.key === "k") {
    event.preventDefault();
    moveHunk(event.key === "j" ? 1 : -1);
  } else if (event.key === "n" || event.key === "p") {
    event.preventDefault();
    moveChapter(event.key === "n" ? 1 : -1);
  } else if (event.key === "c" && state.focusedAnchor) {
    event.preventDefault();
    // Use the stored anchor: re-parsing the display key breaks on paths
    // containing a colon.
    openComposer(state.focusedAnchor, state.focusedLine);
  } else if (event.key === "e" && state.focusedHunk) {
    event.preventDefault();
    const file = state.hunkFile.get(state.focusedHunk);
    if (file) toggleFile(file.path);
  }
});

function moveHunk(delta) {
  const ids = [...document.querySelectorAll(".hunk")].map(el => el.dataset.hunk);
  const current = Math.max(0, ids.indexOf(state.focusedHunk));
  const next = ids[Math.min(ids.length - 1, Math.max(0, current + delta))];
  if (next) scrollToHunk(next);
}

function moveChapter(delta) {
  const ids = state.chapters.map(chapter => chapter.id);
  const current = Math.max(0, ids.indexOf(state.focusedChapter));
  const next = ids[Math.min(ids.length - 1, Math.max(0, current + delta))];
  if (next) scrollToChapter(next);
}


boot();
