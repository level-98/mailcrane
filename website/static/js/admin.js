// /admin (#417): the live state of the repository, from GitHub's public REST
// API. This is the one page on the site that calls a third party at run time;
// website/README.md records the exception.
//
// - GitHub text (titles, names, labels) comes from anyone, so it only ever
//   reaches the page through textContent, never innerHTML.
// - Links are built from validated numbers, or are URLs checked to start with
//   https://github.com/level-98/mailcrane/.
// - Unauthenticated, GitHub allows 60 requests an hour per network, and a 304
//   counts like any other answer. So this page spends at most BUDGET requests
//   per rolling hour in this browser (counted in localStorage, so reloads and
//   other tabs share it): 2 every 5 minutes, 4 every 15 minutes, and a
//   comments request only when an open fleet PR has changed. That is about 40
//   an hour, leaving room for the Refresh button. ETags still save bandwidth.
// - Refreshes only while the tab is visible.
"use strict";

(() => {
  const OWNER = "level-98";
  const API = `https://api.github.com/repos/${OWNER}/mailcrane`;
  const REPO = `https://github.com/${OWNER}/mailcrane`;
  const HOUR = 60 * 60 * 1000;
  const DAY = 24 * HOUR;
  const BUDGET = 50;
  const FAST_MS = 5 * 60 * 1000;
  const SLOW_MS = 15 * 60 * 1000;
  const LOW_REMAINING = 5;
  const PREFIX = "mailcrane-admin:";

  const $ = (id) => document.getElementById(id);
  const statusLine = $("admin-status");
  const errorLine = $("admin-error");
  const refreshButton = $("admin-refresh");
  if (!statusLine || !errorLine || !refreshButton) return;

  // ---------- Small helpers ----------

  const isNumber = (n) => Number.isSafeInteger(n) && n > 0;
  // A commit SHA: hex with at least one letter, so a date like 20261126 isn't one.
  const SHA = /\b(?=[0-9]*[a-f])[0-9a-f]{7,40}\b/;
  const isSha = (s) => typeof s === "string" && /^[0-9a-f]{7,40}$/.test(s) && /[a-f]/.test(s);
  const text = (s) => (typeof s === "string" ? s : "");
  const list = (value) => (Array.isArray(value) ? value : []);
  const names = (labels) => list(labels).map((l) => text(l && l.name));
  const time = (iso) => {
    const t = Date.parse(iso);
    return Number.isFinite(t) ? t : 0;
  };
  const hasNextPage = (response) => /rel="next"/.test(response.headers.get("Link") || "");

  // ---------- Storage (may be blocked: private browsing, strict settings) ----------

  const memory = new Map();

  function load(key, fallback) {
    try {
      const raw = localStorage.getItem(PREFIX + key);
      return raw === null ? fallback : JSON.parse(raw);
    } catch (error) {
      return memory.has(key) ? memory.get(key) : fallback;
    }
  }

  function save(key, value) {
    memory.set(key, value);
    try {
      localStorage.setItem(PREFIX + key, JSON.stringify(value));
    } catch (error) {
      // Full or blocked: this tab keeps going on the in-memory copy.
    }
  }

  // ---------- The request budget: BUDGET per rolling hour, in this browser ----------

  function spent() {
    const now = Date.now();
    return list(load("spent", [])).filter((t) => Number.isFinite(t) && t <= now && now - t < HOUR);
  }

  const budgetLeft = () => BUDGET - spent().length;

  // When `needed` requests fit in the budget again (now, if they already do).
  function budgetFreeAt(needed) {
    const times = spent().sort((a, b) => a - b);
    const over = times.length + needed - BUDGET;
    return over <= 0 ? Date.now() : times[over - 1] + HOUR;
  }

  function spend() {
    const times = spent();
    times.push(Date.now());
    save("spent", times);
  }

  // ---------- Requests ----------

  // Only the fields the page shows are kept, so storage stays small.
  const slimPull = (pr) => ({
    number: pr && pr.number,
    title: text(pr && pr.title),
    draft: Boolean(pr && pr.draft),
    labels: names(pr && pr.labels),
    sha: text(pr && pr.head && pr.head.sha),
    created_at: text(pr && pr.created_at),
    updated_at: text(pr && pr.updated_at),
    merged_at: text(pr && pr.merged_at),
  });
  const slimRun = (run) => ({
    id: run && run.id,
    event: text(run && run.event),
    branch: text(run && run.head_branch),
    sha: text(run && run.head_sha),
    status: text(run && run.status),
    conclusion: text(run && run.conclusion),
    created_at: text(run && run.created_at),
  });

  const ENDPOINTS = {
    open: {
      path: "/pulls?state=open&per_page=100",
      slim: (data, response) => ({ more: hasNextPage(response), items: list(data).map(slimPull) }),
    },
    ci: {
      path: "/actions/workflows/ci.yml/runs?per_page=100",
      slim: (data) => list(data && data.workflow_runs).map(slimRun),
    },
    releases: {
      path: "/releases?per_page=10",
      slim: (data) =>
        list(data).map((r) => ({
          name: text(r && r.name),
          tag: text(r && r.tag_name),
          draft: Boolean(r && r.draft),
          published_at: text(r && r.published_at),
          commitish: text(r && r.target_commitish),
          assets: list(r && r.assets).map((a) => ({ name: text(a && a.name), url: text(a && a.browser_download_url) })),
        })),
    },
    issues: {
      path: "/issues?state=open&per_page=100",
      slim: (data, response) => ({
        more: hasNextPage(response),
        items: list(data)
          .filter((i) => i && !i.pull_request)
          .map((i) => ({
            number: i.number,
            title: text(i.title),
            labels: names(i.labels),
            assigned: list(i.assignees).length > 0,
            created_at: text(i.created_at),
          })),
      }),
    },
    closed: {
      path: "/pulls?state=closed&sort=updated&direction=desc&per_page=50",
      slim: (data, response) => ({ more: hasNextPage(response), items: list(data).map(slimPull) }),
    },
    release: {
      path: "/actions/workflows/release.yml/runs?per_page=3",
      slim: (data) => list(data && data.workflow_runs).map(slimRun),
    },
    // The newest comments in the whole repository, for "reviewed on head".
    comments: {
      path: "/issues/comments?sort=created&direction=desc&per_page=100",
      slim: (data) =>
        list(data).map((c) => ({
          number: Number((text(c && c.issue_url).match(/\/issues\/(\d+)$/) || [])[1]),
          login: text(c && c.user && c.user.login),
          // A review comment names the commit near its start.
          start: text(c && c.body).slice(0, 400),
          created_at: text(c && c.created_at),
        })),
    },
  };
  const FAST = ["open", "ci"];
  const SLOW = ["releases", "issues", "closed", "release"];

  let errors = new Set();
  let remaining = null;

  const pausedUntil = () => Number(load("pausedUntil", 0)) || 0;

  function pauseUntil(at) {
    if (at > pausedUntil()) save("pausedUntil", at);
  }

  function noteRateLimit(response) {
    const left = response.headers.get("X-RateLimit-Remaining");
    const reset = response.headers.get("X-RateLimit-Reset");
    if (left !== null && Number.isFinite(Number(left))) {
      // The lowest figure of this refresh: answers arrive in any order.
      remaining = remaining === null ? Number(left) : Math.min(remaining, Number(left));
      if (Number(left) < LOW_REMAINING && reset !== null && Number.isFinite(Number(reset))) {
        pauseUntil(Number(reset) * 1000);
      }
    }
    const retryAfter = Number(response.headers.get("Retry-After"));
    if (Number.isFinite(retryAfter) && retryAfter > 0) pauseUntil(Date.now() + retryAfter * 1000);
  }

  // Fetches one endpoint and saves its slim data. The page always renders
  // from what's saved; failures go to `errors`.
  async function refreshEndpoint(key) {
    const endpoint = ENDPOINTS[key];
    const stored = load(`data:${key}`, null);
    if (budgetLeft() < 1) {
      errors.add("this browser’s hourly request budget is used up");
      return;
    }
    const headers = { Accept: "application/vnd.github+json" };
    if (stored && typeof stored.etag === "string") headers["If-None-Match"] = stored.etag;
    spend();
    let response;
    try {
      // no-store: the browser's own cache would turn GitHub's 304 into a 200.
      response = await fetch(API + endpoint.path, {
        headers,
        cache: "no-store",
        credentials: "omit",
        referrerPolicy: "no-referrer",
      });
    } catch (error) {
      errors.add("GitHub couldn’t be reached");
      return;
    }
    noteRateLimit(response);
    if (response.status === 304 && stored) {
      save(`data:${key}`, { ...stored, savedAt: Date.now() });
      return;
    }
    if (response.ok) {
      try {
        const data = endpoint.slim(await response.json(), response);
        save(`data:${key}`, { etag: response.headers.get("ETag"), data, savedAt: Date.now() });
      } catch (error) {
        errors.add("GitHub sent something unreadable");
      }
      return;
    }
    if (response.status === 403 || response.status === 429) {
      pauseUntil(Date.now() + 10 * 60 * 1000);
      errors.add("GitHub’s rate limit was reached");
      return;
    }
    errors.add(`GitHub answered ${response.status}`);
  }

  const saved = (key) => {
    const entry = load(`data:${key}`, null);
    return entry ? entry.data : null;
  };

  // ---------- Reviews: whether the fleet's review seat commented on the head ----------

  // Fetched only when an open fleet PR's head or update time changed since the
  // last look (a new comment changes its update time).
  async function refreshReviews() {
    const open = saved("open");
    const fleet = list(open && open.items).filter((pr) => pr.labels.includes("fleet") && isNumber(pr.number));
    const known = load("reviews", {});
    const changed = fleet.some((pr) => {
      const seen = known[pr.number];
      return !seen || seen.sha !== pr.sha || seen.updated !== pr.updated_at;
    });
    if (!changed) return;
    await refreshEndpoint("comments");
    const comments = saved("comments");
    if (comments === null) return;
    // Fewer than a page means this is every comment there is.
    const complete = comments.length < 100;
    const oldest = comments.length > 0 ? time(comments[comments.length - 1].created_at) : 0;
    const reviews = {};
    fleet.forEach((pr) => {
      // Only the repository owner's own comments count (the fleet posts as
      // the owner); anyone else could type the same prefix.
      const mine = comments.filter(
        (c) => c.number === pr.number && c.login === OWNER && c.start.startsWith("**mailbot.review:**"),
      );
      let state;
      if (mine.some((c) => isSha(pr.sha) && c.start.includes(pr.sha.slice(0, 7)))) state = "head";
      else if (mine.length > 0) state = "earlier";
      else if (complete || oldest <= time(pr.created_at)) state = "none";
      else state = "unknown";
      reviews[pr.number] = { sha: pr.sha, updated: pr.updated_at, state };
    });
    save("reviews", reviews);
  }

  // ---------- Building the page (textContent only) ----------

  function node(tag, className, content) {
    const element = document.createElement(tag);
    if (className) element.className = className;
    if (content !== undefined && content !== null) element.textContent = String(content);
    return element;
  }

  // A link only to the repository on github.com; anything else becomes text.
  function link(href, content, className) {
    if (typeof href !== "string" || !href.startsWith(`${REPO}/`)) return node("span", className, content);
    const a = node("a", className, content);
    a.href = href;
    a.rel = "noreferrer";
    return a;
  }

  function when(iso) {
    const t = time(iso);
    if (!t) return node("span", "admin-muted", "");
    const minutes = Math.round((Date.now() - t) / 60000);
    let ago;
    if (minutes < 1) ago = "just now";
    else if (minutes < 60) ago = `${minutes} min ago`;
    else if (minutes < 48 * 60) ago = `${Math.round(minutes / 60)} h ago`;
    else ago = `${Math.round(minutes / 1440)} days ago`;
    const span = node("time", "admin-muted", ago);
    span.dateTime = new Date(t).toISOString();
    span.title = new Date(t).toLocaleString();
    return span;
  }

  const pill = (label, tone) => node("span", `admin-pill admin-pill-${tone}`, label);

  // A workflow run's state; `prefix` names it ("CI") where that isn't obvious.
  function runPill(run, prefix = "") {
    const say = (state) => (prefix ? `${prefix} ${state}` : state);
    if (!run) return pill(say("unknown"), "neutral");
    if (run.status !== "completed") return pill(say(run.status === "queued" ? "queued" : "running"), "running");
    if (run.conclusion === "success") return pill(say("passed"), "ok");
    if (run.conclusion === "skipped" || run.conclusion === "neutral") return pill(say(run.conclusion), "neutral");
    return pill(say(run.conclusion || "failed"), "fail");
  }

  function fill(id, children, emptyText, note) {
    const target = $(id);
    if (!target) return;
    target.replaceChildren();
    if (children === null) {
      target.append(node("p", "admin-empty", "Not available yet."));
      return;
    }
    if (children.length === 0) {
      target.append(node("p", "admin-empty", emptyText));
    } else {
      const ul = node("ul", "admin-list");
      children.forEach((child) => ul.append(child));
      target.append(ul);
    }
    if (note) target.append(node("p", "admin-meta", note));
  }

  function row(...parts) {
    const li = node("li", "admin-row");
    parts.filter(Boolean).forEach((part) => li.append(part));
    return li;
  }

  function numbered(kind, item) {
    const wrap = node("span", "admin-title");
    if (isNumber(item.number)) wrap.append(link(`${REPO}/${kind}/${item.number}`, `#${item.number}`, "admin-num"), " ");
    wrap.append(item.title);
    return wrap;
  }

  function renderBuild() {
    const target = $("admin-build");
    if (!target) return;
    target.replaceChildren();
    const releases = saved("releases");
    if (releases === null) {
      target.append(node("p", "admin-empty", "Not available yet."));
      return;
    }
    const build = list(releases).find((r) => !r.draft && r.tag.startsWith("test-build-"));
    if (!build) {
      target.append(node("p", "admin-empty", "No test build yet."));
      return;
    }
    const apk = list(build.assets).find((a) => a.name.endsWith(".apk") && a.url.startsWith(`${REPO}/releases/download/`));
    if (apk) target.append(link(apk.url, `Download ${apk.name}`, "button button-primary admin-download"));
    const meta = node("p", "admin-meta");
    meta.append(node("span", null, build.name || build.tag), " · ", when(build.published_at));
    const commit = isSha(build.commitish) ? build.commitish : (build.name.match(SHA) || [])[0];
    if (isSha(commit)) meta.append(" · ", link(`${REPO}/commit/${commit}`, commit.slice(0, 7), "admin-num"));
    target.append(meta);
  }

  function renderPrs() {
    const open = saved("open");
    if (open === null) return fill("admin-prs", null);
    const ci = list(saved("ci"));
    const reviews = load("reviews", {});
    const fleet = list(open.items).filter((pr) => pr.labels.includes("fleet"));
    fill(
      "admin-prs",
      fleet.map((pr) => {
        const badges = node("span", "admin-badges");
        badges.append(pr.draft ? pill("draft", "neutral") : pill("ready", "running"));
        badges.append(runPill(ci.find((r) => r.sha === pr.sha), "CI"));
        const review = reviews[pr.number];
        const state = review && review.sha === pr.sha ? review.state : "unknown";
        const label = { head: "reviewed on head", earlier: "reviewed earlier", none: "not reviewed" }[state];
        badges.append(pill(label || "review unknown", state === "head" ? "ok" : "neutral"));
        if (pr.labels.includes("qa-verified")) badges.append(pill("qa-verified", "ok"));
        if (pr.labels.includes("security")) badges.append(pill("security", "fail"));
        badges.append(when(pr.created_at));
        return row(numbered("pull", pr), badges);
      }),
      "No open fleet pull requests.",
      open.more ? "More than 100 pull requests are open; only the newest 100 are checked." : null,
    );
  }

  function renderMerged() {
    const closed = saved("closed");
    if (closed === null) return fill("admin-merged", null);
    const merged = list(closed.items)
      .filter((pr) => pr.labels.includes("fleet") && pr.merged_at && Date.now() - time(pr.merged_at) < DAY)
      .sort((a, b) => time(b.merged_at) - time(a.merged_at));
    fill(
      "admin-merged",
      merged.map((pr) => row(numbered("pull", pr), when(pr.merged_at))),
      "Nothing merged in the last 24 hours.",
      closed.more ? "Only the 50 most recently updated closed pull requests are checked." : null,
    );
  }

  function renderRuns(id, runs) {
    if (runs === null) return fill(id, null);
    fill(
      id,
      runs.slice(0, 3).map((run) => {
        const title = node("span", "admin-title");
        const label = `${run.event} on ${run.branch || "?"}`;
        title.append(isNumber(run.id) ? link(`${REPO}/actions/runs/${run.id}`, label) : node("span", null, label));
        const badges = node("span", "admin-badges");
        badges.append(runPill(run), when(run.created_at));
        return row(title, badges);
      }),
      "No runs yet.",
    );
  }

  function renderIssues() {
    const target = $("admin-issues");
    const issues = saved("issues");
    if (!target) return;
    if (issues === null) return fill("admin-issues", null);
    const open = list(issues.items);
    const count = (predicate) => open.filter(predicate).length;
    // With more than one page, the counts are only a lower bound.
    const atLeast = issues.more ? "≥ " : "";
    const counts = node("p", "admin-counts");
    const stat = (n, label) => {
      const s = node("span", "admin-stat");
      s.append(node("strong", null, `${atLeast}${n}`), ` ${label}`);
      return s;
    };
    counts.append(
      stat(count((i) => i.labels.includes("agent-ready") && !i.assigned), "agent-ready, free"),
      stat(count((i) => i.labels.includes("needs-human")), "needs-human"),
      stat(count((i) => !i.labels.some((l) => l.startsWith("area:"))), "untriaged"),
    );
    const newest = open
      .slice()
      .sort((a, b) => time(b.created_at) - time(a.created_at))
      .slice(0, 5)
      .map((issue) => row(numbered("issues", issue), when(issue.created_at)));
    fill("admin-issues", newest, "No open issues.");
    target.prepend(counts);
  }

  function renderAll() {
    renderBuild();
    renderPrs();
    renderMerged();
    const ci = saved("ci");
    renderRuns("admin-ci", ci === null ? null : list(ci).filter((r) => r.event === "push" && r.branch === "main"));
    const release = saved("release");
    renderRuns("admin-release", release === null ? null : list(release));
    renderIssues();
  }

  // ---------- Scheduling ----------

  const lastRun = (tier) => Number(load(`last:${tier}`, 0)) || 0;

  function clock(ms) {
    return new Date(ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  }

  function countdown(ms) {
    const s = Math.max(0, Math.ceil(ms / 1000));
    return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
  }

  // When the next automatic refresh happens: the next tier due, no earlier
  // than the budget and any rate-limit pause allow.
  function nextDue() {
    const fast = Math.max(lastRun("fast") + FAST_MS, budgetFreeAt(FAST.length));
    const slow = Math.max(lastRun("slow") + SLOW_MS, budgetFreeAt(SLOW.length));
    return Math.max(Math.min(fast, slow), pausedUntil());
  }

  let running = false;
  let lastOutcome = "";

  function showStatus() {
    const parts = [];
    if (lastOutcome) parts.push(lastOutcome);
    const now = Date.now();
    if (pausedUntil() > now) parts.push(`paused until ${clock(pausedUntil())} (GitHub’s limit)`);
    else if (!running) parts.push(`next refresh in ${countdown(nextDue() - now)}`);
    parts.push(`${budgetLeft()} of ${BUDGET} requests left this hour`);
    if (remaining !== null) parts.push(`GitHub: ${remaining} left for this network`);
    statusLine.textContent = parts.join(" · ");
    // The button spends the fast tier's requests: only when the budget has them.
    const wait = Math.max(budgetFreeAt(FAST.length), pausedUntil()) - now;
    refreshButton.disabled = running || wait > 0;
    refreshButton.textContent = wait > 0 ? `Refresh (${countdown(wait)})` : "Refresh";
  }

  async function refresh(manual) {
    if (running || pausedUntil() > Date.now()) return;
    running = true;
    errors = new Set();
    remaining = null;
    showStatus();
    const now = Date.now();
    try {
      const fastDue = manual || now - lastRun("fast") >= FAST_MS;
      const slowDue = now - lastRun("slow") >= SLOW_MS;
      if (fastDue && budgetLeft() >= FAST.length) {
        await Promise.all(FAST.map(refreshEndpoint));
        save("last:fast", Date.now());
        await refreshReviews();
      }
      if (slowDue && budgetLeft() >= SLOW.length) {
        await Promise.all(SLOW.map(refreshEndpoint));
        save("last:slow", Date.now());
      }
    } finally {
      running = false;
    }
    renderAll();
    if (errors.size > 0) {
      const times = Object.keys(ENDPOINTS)
        .map((key) => load(`data:${key}`, null))
        .filter(Boolean)
        .map((entry) => entry.savedAt)
        .filter(Number.isFinite);
      const savedNote =
        times.length === 0 ? "Nothing is saved in this browser yet." : `Showing what was saved by ${clock(Math.min(...times))}.`;
      errorLine.hidden = false;
      errorLine.textContent = `${[...errors].join("; ")}. ${savedNote}`;
      lastOutcome = `Tried at ${clock(Date.now())}`;
    } else {
      errorLine.hidden = true;
      errorLine.textContent = "";
      lastOutcome = `Updated at ${clock(Date.now())}`;
    }
    showStatus();
  }

  // Forget keys from older versions of this page (whole raw responses).
  try {
    const keep = new Set(["spent", "pausedUntil", "reviews", "last:fast", "last:slow"]);
    Object.keys(ENDPOINTS).forEach((key) => keep.add(`data:${key}`));
    for (let i = localStorage.length - 1; i >= 0; i -= 1) {
      const key = localStorage.key(i);
      if (key && key.startsWith(PREFIX) && !keep.has(key.slice(PREFIX.length))) localStorage.removeItem(key);
    }
  } catch (error) {
    // Storage blocked: nothing to clean.
  }

  refreshButton.addEventListener("click", () => refresh(true));

  // Once a second while visible: update the countdown, refresh when due.
  let ticker = null;
  function tick() {
    if (!running && Date.now() >= nextDue()) refresh(false);
    else showStatus();
  }
  function start() {
    clearInterval(ticker);
    if (document.hidden) return;
    ticker = setInterval(tick, 1000);
    tick();
  }
  document.addEventListener("visibilitychange", start);

  // Show what's saved at once, then refresh whatever is due.
  renderAll();
  start();
})();
