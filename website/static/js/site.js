const themeToggle = document.querySelector("#theme-toggle");
const themeColor = document.querySelector('meta[name="theme-color"]');

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

const systemColorScheme = window.matchMedia("(prefers-color-scheme: dark)");
const themePreference = localStorage.getItem("mailcrane-theme");
if (themePreference !== "light" && themePreference !== "dark") {
  systemColorScheme.addEventListener("change", (event) => {
    applyTheme(event.matches ? "dark" : "light");
  });
}
