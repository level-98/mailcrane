const REPOSITORY = "level-98/mailcrane";
const MILESTONE_INFO = {
  M0: { name: "Foundations", description: "Set up the shared core, app shells, and contributor workflow." },
  M1: { name: "Read mail", description: "Connect an account and read your inbox, online or offline." },
  M2: { name: "Send mail", description: "Compose, reply, and reliably deliver your messages." },
  M3: { name: "Providers & sign-in", description: "Make setup straightforward across popular providers." },
  M4: { name: "Everyday features", description: "Bring together the thoughtful tools that make email better." },
  M5: { name: "Privacy & security", description: "Keep personal mail protected and in your control." },
  M6: { name: "Release", description: "Test, package, and bring Mailcrane to its first users." },
  P: { name: "Before public launch", description: "Resolve the project, store, domain, and naming checks before going public." }
};

const themeToggle = document.querySelector("#theme-toggle");
const themeColor = document.querySelector('meta[name="theme-color"]');
const systemColorScheme = window.matchMedia("(prefers-color-scheme: dark)");
const themePreference = localStorage.getItem("mailcrane-theme");

function applyTheme(theme, persist = false) {
  const isDark = theme === "dark";
  document.documentElement.dataset.theme = theme;
  document.documentElement.style.colorScheme = theme;
  themeToggle.setAttribute("aria-pressed", String(isDark));
  themeToggle.setAttribute("aria-label", `Switch to ${isDark ? "light" : "dark"} theme`);
  themeToggle.title = `Switch to ${isDark ? "light" : "dark"} theme`;
  themeColor.content = isDark ? "#0F1424" : "#F4F6FB";

  document.querySelectorAll(".brand-wordmark").forEach((wordmark) => {
    wordmark.src = isDark ? wordmark.dataset.darkSrc : wordmark.dataset.lightSrc;
  });

  if (persist) {
    localStorage.setItem("mailcrane-theme", theme);
  }
}

applyTheme(document.documentElement.dataset.theme === "dark" ? "dark" : "light");
themeToggle.addEventListener("click", () => {
  applyTheme(document.documentElement.dataset.theme === "dark" ? "light" : "dark", true);
});

if (themePreference !== "light" && themePreference !== "dark") {
  systemColorScheme.addEventListener("change", (event) => {
    applyTheme(event.matches ? "dark" : "light");
  });
}

const milestoneContainer = document.querySelector("#milestones");
const statusMessage = document.querySelector("#roadmap-status");
const refreshButton = document.querySelector("#refresh-roadmap");
const overallCount = document.querySelector("#overall-count");
const overallBar = document.querySelector("#overall-bar");
const overallProgress = document.querySelector(".overview-bar");
const overallPercent = document.querySelector("#overall-percent");
const updatedAt = document.querySelector("#updated-at");
let loading = false;

function setStatus(message, state) {
  statusMessage.textContent = message;
  statusMessage.dataset.state = state || "info";
}

async function fetchIssues() {
  const issues = [];
  let page = 1;

  while (true) {
    const url = `https://api.github.com/repos/${REPOSITORY}/issues?state=all&per_page=100&page=${page}`;
    const response = await fetch(url, {
      headers: { Accept: "application/vnd.github+json" }
    });

    if (!response.ok) {
      throw new Error(`GitHub returned ${response.status} ${response.statusText}.`);
    }

    const batch = await response.json();
    if (!Array.isArray(batch)) {
      throw new Error("GitHub returned an unexpected issue list.");
    }

    issues.push(...batch.filter((issue) => !issue.pull_request));
    if (batch.length < 100) {
      return issues;
    }
    page += 1;
  }
}

function taskId(title) {
  const match = /^\[((?:M[0-6])|P)-(\d{2})\]\s+/i.exec(title);
  return match ? { milestone: match[1].toUpperCase(), sequence: Number(match[2]) } : null;
}

function createTaskItem(issue) {
  const row = document.createElement("li");
  const state = document.createElement("span");
  const link = document.createElement("a");

  state.className = `task-state${issue.state === "closed" ? " is-done" : ""}`;
  state.textContent = issue.state === "closed" ? "DONE" : "OPEN";
  link.href = `https://github.com/${REPOSITORY}/issues/${issue.number}`;
  link.textContent = issue.title.replace(/^\[[^\]]+\]\s*/, "");
  link.target = "_blank";
  link.rel = "noopener noreferrer";

  row.append(state, link);
  return row;
}

function createMilestoneCard(key, issues) {
  const info = MILESTONE_INFO[key];
  const completed = issues.filter((issue) => issue.state === "closed").length;
  const percent = issues.length ? Math.round((completed / issues.length) * 100) : 0;
  const card = document.createElement("article");
  card.className = "milestone-card";

  const top = document.createElement("div");
  top.className = "milestone-topline";
  const name = document.createElement("div");
  name.className = "milestone-name";
  const badge = document.createElement("span");
  badge.className = "milestone-id";
  badge.textContent = key;
  const heading = document.createElement("h3");
  heading.textContent = info.name;
  name.append(badge, heading);
  const count = document.createElement("span");
  count.className = "milestone-count";
  count.textContent = `${completed} / ${issues.length} complete`;
  top.append(name, count);

  const description = document.createElement("p");
  description.className = "milestone-description";
  description.textContent = info.description;

  const progress = document.createElement("div");
  progress.className = "milestone-bar";
  progress.setAttribute("role", "progressbar");
  progress.setAttribute("aria-label", `${key} ${info.name} progress`);
  progress.setAttribute("aria-valuemin", "0");
  progress.setAttribute("aria-valuemax", "100");
  progress.setAttribute("aria-valuenow", String(percent));
  const fill = document.createElement("span");
  fill.style.width = `${percent}%`;
  progress.append(fill);

  const details = document.createElement("details");
  details.className = "milestone-details";
  const summary = document.createElement("summary");
  summary.textContent = "View tasks";
  const list = document.createElement("ul");
  list.className = "task-list";
  issues.forEach((issue) => list.append(createTaskItem(issue)));
  details.append(summary, list);

  card.append(top, description, progress, details);
  return card;
}

function renderRoadmap(issues) {
  const tasksByMilestone = Object.fromEntries(
    Object.keys(MILESTONE_INFO).map((key) => [key, []])
  );

  issues.forEach((issue) => {
    if (typeof issue.title !== "string" || !Number.isInteger(issue.number)) {
      return;
    }
    const id = taskId(issue.title);
    if (id && tasksByMilestone[id.milestone]) {
      tasksByMilestone[id.milestone].push(issue);
    }
  });

  const taskMilestones = Object.entries(tasksByMilestone)
    .filter(([, tasks]) => tasks.length > 0);
  const allTasks = taskMilestones.flatMap(([, tasks]) => tasks);
  const completed = allTasks.filter((issue) => issue.state === "closed").length;
  const percent = allTasks.length ? Math.round((completed / allTasks.length) * 100) : 0;

  milestoneContainer.replaceChildren(
    ...taskMilestones.map(([key, tasks]) => {
      tasks.sort((a, b) => taskId(a.title).sequence - taskId(b.title).sequence);
      return createMilestoneCard(key, tasks);
    })
  );

  overallCount.textContent = `${completed} of ${allTasks.length} tasks complete`;
  overallPercent.textContent = `${percent}%`;
  overallBar.style.width = `${percent}%`;
  overallProgress.setAttribute("aria-valuenow", String(percent));
  updatedAt.textContent = `Updated ${new Intl.DateTimeFormat(undefined, {
    hour: "numeric",
    minute: "2-digit"
  }).format(new Date())}`;
  setStatus(`Showing ${allTasks.length} roadmap issues across ${taskMilestones.length} groups.`, "success");
}

async function loadRoadmap() {
  if (loading) {
    return;
  }

  loading = true;
  refreshButton.disabled = true;
  setStatus("Loading milestone progress from GitHub…");

  try {
    const issues = await fetchIssues();
    renderRoadmap(issues);
  } catch (error) {
    setStatus(
      `Couldn’t load live progress: ${error.message} Check the project’s issues on GitHub.`,
      "error"
    );
    updatedAt.textContent = "Progress unavailable";
  } finally {
    loading = false;
    refreshButton.disabled = false;
  }
}

refreshButton.addEventListener("click", loadRoadmap);
loadRoadmap();
window.setInterval(loadRoadmap, 5 * 60 * 1000);
