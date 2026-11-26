// /admin (#417): the live state of the repository, from GitHub's public REST
// API. This is the one page on the site that calls a third party at run time;
// website/README.md records the exception.
//
// - GitHub text (titles, names, labels) comes from anyone, so it only ever
//   reaches the page through textContent, never innerHTML.
// - Links are built from validated numbers, or are URLs checked to start with
//   https://github.com/level-98/mailcrane/.
// - Unauthenticated: 60 requests an hour per IP. Every request is conditional
//   (If-None-Match with the stored ETag) and a 304 doesn't count, so an
//   unchanged repository costs nothing. Responses are kept in localStorage.
// - Refreshes every 60 s while the tab is visible, and backs off until the
//   limit resets when few requests are left.
"use strict";

(() => {
  const OWNER = "level-98";
  const API = `https://api.github.com/repos/${OWNER}/mailcrane`;
  const REPO = `https://github.com/${OWNER}/mailcrane`;
  const CI_WORKFLOW = ".github/workflows/ci.yml";
  const REFRESH_MS = 60 * 1000;
  const LOW_REMAINING = 10;
  const DAY_MS = 24 * 60 * 60 * 1000;
  const CACHE_PREFIX = "mailcrane-admin:";

  const $ = (id) => document.getElementById(id);
  const statusLine = $("admin-status");
  const errorLine = $("admin-error");
  const refreshButton = $("admin-refresh");
  if (!statusLine || !errorLine || !refreshButton) return;

  let pausedUntil = 0;
  let remaining = null;
  let lastRefresh = 0;
  let running = false;
  let timer = null;

  // ---------- Storage (may be blocked: private browsing, strict settings) ----------

  function cacheGet(url) {
    try {
      const raw = localStorage.getItem(CACHE_PREFIX + url);
      return raw ? JSON.parse(raw) : null;
    } catch (error) {
      return null;
    }
  }

  function cachePut(url, entry) {
    try {
      localStorage.setItem(CACHE_PREFIX + url, JSON.stringify(entry));
    } catch (error) {
      // Full or blocked: the page still works, it just can't revalidate.
    }
  }

  // ---------- Requests ----------

  function noteRateLimit(response) {
    const left = Number(response.headers.get("X-RateLimit-Remaining"));
    const reset = Number(response.headers.get("X-RateLimit-Reset"));
    if (Number.isFinite(left)) remaining = left;
    if (Number.isFinite(left) && Number.isFinite(reset) && left < LOW_REMAINING) {
      pausedUntil = Math.max(pausedUntil, reset * 1000);
    }
    // Secondary limits answer with Retry-After (seconds).
    const retryAfter = Number(response.headers.get("Retry-After"));
    if (Number.isFinite(retryAfter) && retryAfter > 0) {
      pausedUntil = Math.max(pausedUntil, Date.now() + retryAfter * 1000);
    }
  }

  // Returns { data, savedAt, error }: fresh data, or the saved copy with an
  // error when GitHub can't be reached or refuses.
  async function get(path) {
    const url = API + path;
    const cached = cacheGet(url);
    const fallback = (error) => ({
      data: cached ? cached.data : null,
      savedAt: cached ? cached.savedAt : null,
      error,
    });
    const headers = { Accept: "application/vnd.github+json" };
    if (cached && typeof cached.etag === "string") headers["If-None-Match"] = cached.etag;
    let response;
    try {
      // no-store: the browser's own cache would turn GitHub's 304 into a 200.
      response = await fetch(url, {
        headers,
        cache: "no-store",
        credentials: "omit",
        referrerPolicy: "no-referrer",
      });
    } catch (error) {
      return fallback("GitHub couldn’t be reached");
    }
    noteRateLimit(response);
    if (response.status === 304 && cached) {
      return { data: cached.data, savedAt: Date.now(), error: null };
    }
    if (response.ok) {
      let data;
      try {
        data = await response.json();
      } catch (error) {
        return fallback("GitHub sent something unreadable");
      }
      cachePut(url, { etag: response.headers.get("ETag"), data, savedAt: Date.now() });
      return { data, savedAt: Date.now(), error: null };
    }
    if (response.status === 403 || response.status === 429) {
      if (pausedUntil <= Date.now()) pausedUntil = Date.now() + 5 * 60 * 1000;
      return fallback("GitHub’s rate limit was reached");
    }
    return fallback(`GitHub answered ${response.status}`);
  }

  // ---------- Building the page (textContent only) ----------

  function node(tag, className, text) {
    const element = document.createElement(tag);
    if (className) element.className = className;
    if (text !== undefined && text !== null) element.textContent = String(text);
    return element;
  }

  // A link only to the repository on github.com; anything else becomes text.
  function link(href, text, className) {
    if (typeof href !== "string" || !href.startsWith(`${REPO}/`)) return node("span", className, text);
    const a = node("a", className, text);
    a.href = href;
    a.rel = "noreferrer";
    return a;
  }

  const isNumber = (n) => Number.isSafeInteger(n) && n > 0;
  const isSha = (s) => typeof s === "string" && /^[0-9a-f]{7,40}$/.test(s);
  const text = (s) => (typeof s === "string" ? s : "");
  const list = (value) => (Array.isArray(value) ? value : []);
  const labelNames = (item) => list(item && item.labels).map((l) => text(l && l.name));

  function when(iso) {
    const time = Date.parse(iso);
    if (!Number.isFinite(time)) return node("span", "admin-muted", "");
    const minutes = Math.round((Date.now() - time) / 60000);
    let ago;
    if (minutes < 1) ago = "just now";
    else if (minutes < 60) ago = `${minutes} min ago`;
    else if (minutes < 48 * 60) ago = `${Math.round(minutes / 60)} h ago`;
    else ago = `${Math.round(minutes / 1440)} days ago`;
    const span = node("time", "admin-muted", ago);
    span.dateTime = new Date(time).toISOString();
    span.title = new Date(time).toLocaleString();
    return span;
  }

  function pill(label, tone) {
    return node("span", `admin-pill admin-pill-${tone}`, label);
  }

  // A workflow run's state; `prefix` names it ("CI") where that isn't obvious.
  function runPill(run, prefix = "") {
    const say = (state) => (prefix ? `${prefix} ${state}` : state);
    if (!run) return pill(say("not run yet"), "neutral");
    const status = text(run.status);
    if (status !== "completed") return pill(say(status === "queued" ? "queued" : "running"), "running");
    const conclusion = text(run.conclusion);
    if (conclusion === "success") return pill(say("passed"), "ok");
    if (conclusion === "skipped" || conclusion === "neutral") return pill(say(conclusion), "neutral");
    return pill(say(conclusion || "failed"), "fail");
  }

  function fill(id, children, emptyText) {
    const target = $(id);
    if (!target) return;
    target.replaceChildren();
    if (children === null) {
      target.append(node("p", "admin-empty", "Not available yet."));
    } else if (children.length === 0) {
      target.append(node("p", "admin-empty", emptyText));
    } else {
      const ul = node("ul", "admin-list");
      children.forEach((child) => ul.append(child));
      target.append(ul);
    }
  }

  function row(...parts) {
    const li = node("li", "admin-row");
    parts.filter(Boolean).forEach((part) => li.append(part));
    return li;
  }

  function prLink(pr) {
    if (!isNumber(pr && pr.number)) return node("span", "admin-title", text(pr && pr.title));
    const wrap = node("span", "admin-title");
    wrap.append(link(`${REPO}/pull/${pr.number}`, `#${pr.number}`, "admin-num"), " ", text(pr.title));
    return wrap;
  }

  function renderBuild(releases) {
    const target = $("admin-build");
    if (!target) return;
    target.replaceChildren();
    if (releases === null) {
      target.append(node("p", "admin-empty", "Not available yet."));
      return;
    }
    const build = list(releases).find((r) => r && !r.draft && text(r.tag_name).startsWith("test-build-"));
    if (!build) {
      target.append(node("p", "admin-empty", "No test build yet."));
      return;
    }
    const apk = list(build.assets).find(
      (a) => a && text(a.name).endsWith(".apk") && text(a.browser_download_url).startsWith(`${REPO}/releases/download/`),
    );
    if (apk) {
      target.append(link(apk.browser_download_url, `Download ${text(apk.name)}`, "button button-primary admin-download"));
    }
    const meta = node("p", "admin-meta");
    meta.append(node("span", null, text(build.name) || text(build.tag_name)), " · ", when(build.published_at));
    const commit = (text(build.name).match(/\b[0-9a-f]{7,40}\b/) || [])[0];
    if (isSha(commit)) meta.append(" · ", link(`${REPO}/commit/${commit}`, commit, "admin-num"));
    target.append(meta);
  }

  function renderPrs(pulls, runs, reviews) {
    if (pulls === null) return fill("admin-prs", null);
    const fleet = list(pulls).filter((pr) => labelNames(pr).includes("fleet"));
    const ciRuns = list(runs && runs.workflow_runs).filter((r) => r && r.path === CI_WORKFLOW);
    fill(
      "admin-prs",
      fleet.map((pr) => {
        const sha = text(pr.head && pr.head.sha);
        const labels = labelNames(pr);
        const badges = node("span", "admin-badges");
        badges.append(pr.draft ? pill("draft", "neutral") : pill("ready", "running"));
        badges.append(runPill(ciRuns.find((r) => r.head_sha === sha), "CI"));
        const review = reviews.get(pr.number);
        if (review === "head") badges.append(pill("reviewed on head", "ok"));
        else if (review === "earlier") badges.append(pill("reviewed earlier", "neutral"));
        else if (review === "none") badges.append(pill("not reviewed", "neutral"));
        if (labels.includes("qa-verified")) badges.append(pill("qa-verified", "ok"));
        if (labels.includes("security")) badges.append(pill("security", "fail"));
        badges.append(when(pr.created_at));
        return row(prLink(pr), badges);
      }),
      "No open fleet pull requests.",
    );
  }

  function renderMerged(closed) {
    if (closed === null) return fill("admin-merged", null);
    const merged = list(closed)
      .filter((pr) => labelNames(pr).includes("fleet") && Date.now() - Date.parse(pr.merged_at) < DAY_MS)
      .sort((a, b) => Date.parse(b.merged_at) - Date.parse(a.merged_at));
    fill(
      "admin-merged",
      merged.map((pr) => row(prLink(pr), when(pr.merged_at))),
      "Nothing merged in the last 24 hours.",
    );
  }

  function renderRuns(id, runs) {
    if (runs === null) return fill(id, null);
    fill(
      id,
      list(runs && runs.workflow_runs).slice(0, 3).map((run) => {
        const title = node("span", "admin-title");
        const label = `${text(run.event)} on ${text(run.head_branch) || "?"}`;
        title.append(isNumber(run.id) ? link(`${REPO}/actions/runs/${run.id}`, label) : node("span", null, label));
        const badges = node("span", "admin-badges");
        badges.append(runPill(run), when(run.created_at));
        return row(title, badges);
      }),
      "No runs yet.",
    );
  }

  function renderIssues(issues) {
    const target = $("admin-issues");
    if (!target) return;
    if (issues === null) return fill("admin-issues", null);
    const open = list(issues).filter((i) => i && !i.pull_request);
    const count = (predicate) => open.filter(predicate).length;
    const counts = node("p", "admin-counts");
    const stat = (n, label) => {
      const s = node("span", "admin-stat");
      s.append(node("strong", null, n), ` ${label}`);
      return s;
    };
    counts.append(
      stat(count((i) => labelNames(i).includes("agent-ready") && list(i.assignees).length === 0), "agent-ready, free"),
      stat(count((i) => labelNames(i).includes("needs-human")), "needs-human"),
      stat(count((i) => !labelNames(i).some((l) => l.startsWith("area:"))), "untriaged"),
    );
    const newest = open
      .slice()
      .sort((a, b) => Date.parse(b.created_at) - Date.parse(a.created_at))
      .slice(0, 5)
      .map((issue) => {
        const title = node("span", "admin-title");
        if (isNumber(issue.number)) {
          title.append(link(`${REPO}/issues/${issue.number}`, `#${issue.number}`, "admin-num"), " ");
        }
        title.append(text(issue.title));
        return row(title, when(issue.created_at));
      });
    fill("admin-issues", newest, "No open issues.");
    target.prepend(counts);
  }

  // Whether the fleet's review seat has commented on the head commit. Only the
  // repository owner's own comments count (the fleet posts as the owner);
  // anyone else could type the same prefix.
  function reviewState(comments, sha) {
    const fromReview = list(comments).filter(
      (c) => c && c.user && c.user.login === OWNER && text(c.body).startsWith("**mailbot.review:**"),
    );
    if (fromReview.length === 0) return "none";
    return isSha(sha) && fromReview.some((c) => text(c.body).includes(sha.slice(0, 7))) ? "head" : "earlier";
  }

  // ---------- Refreshing ----------

  function clock(ms) {
    return new Date(ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  }

  function showStatus(message) {
    const left = remaining === null ? "" : ` · ${remaining} GitHub requests left this hour`;
    statusLine.textContent = message + left;
  }

  async function refresh() {
    if (running) return;
    if (Date.now() < pausedUntil) {
      showStatus(`Paused until ${clock(pausedUntil)}: GitHub’s hourly limit is nearly used up`);
      return;
    }
    running = true;
    refreshButton.disabled = true;
    const errors = new Set();
    let oldest = null;
    const load = async (path) => {
      const result = await get(path);
      if (result.error) {
        errors.add(result.error);
        if (result.savedAt) oldest = Math.min(oldest === null ? Date.now() : oldest, result.savedAt);
      }
      return result.data;
    };
    try {
      const [releases, pulls, runs, mainRuns, releaseRuns, closed, issues] = await Promise.all([
        load("/releases?per_page=20"),
        load("/pulls?state=open&per_page=50"),
        load("/actions/runs?per_page=50"),
        load("/actions/workflows/ci.yml/runs?branch=main&per_page=3"),
        load("/actions/workflows/release.yml/runs?per_page=3"),
        load("/pulls?state=closed&sort=updated&direction=desc&per_page=30"),
        load("/issues?state=open&per_page=100"),
      ]);
      // One comments request per open fleet PR, to see whether the head was
      // reviewed; unchanged threads answer 304.
      const reviews = new Map();
      const fleet = list(pulls).filter((pr) => labelNames(pr).includes("fleet") && isNumber(pr.number));
      await Promise.all(
        fleet.map(async (pr) => {
          const comments = await load(`/issues/${pr.number}/comments?per_page=100`);
          if (comments !== null) reviews.set(pr.number, reviewState(comments, text(pr.head && pr.head.sha)));
        }),
      );
      renderBuild(releases);
      renderPrs(pulls, runs, reviews);
      renderMerged(closed);
      renderRuns("admin-ci", mainRuns);
      renderRuns("admin-release", releaseRuns);
      renderIssues(issues);
      lastRefresh = Date.now();
      if (errors.size > 0) {
        errorLine.hidden = false;
        const saved = oldest === null ? "Nothing is saved in this browser yet." : `Showing what was saved at ${clock(oldest)}.`;
        errorLine.textContent = `${[...errors].join("; ")}. ${saved}`;
        showStatus(`Tried at ${clock(lastRefresh)}`);
      } else {
        errorLine.hidden = true;
        errorLine.textContent = "";
        showStatus(`Updated at ${clock(lastRefresh)}`);
      }
      if (Date.now() < pausedUntil) {
        showStatus(`Updated at ${clock(lastRefresh)}; paused until ${clock(pausedUntil)} (GitHub’s hourly limit)`);
      }
    } finally {
      running = false;
      refreshButton.disabled = false;
    }
  }

  function schedule() {
    clearTimeout(timer);
    if (document.hidden) return;
    timer = setTimeout(async () => {
      await refresh();
      schedule();
    }, REFRESH_MS);
  }

  refreshButton.addEventListener("click", async () => {
    await refresh();
    schedule();
  });

  document.addEventListener("visibilitychange", async () => {
    if (document.hidden) {
      clearTimeout(timer);
      return;
    }
    if (Date.now() - lastRefresh >= REFRESH_MS) await refresh();
    schedule();
  });

  refresh().then(schedule);
})();
