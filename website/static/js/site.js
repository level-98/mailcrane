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
    try {
      localStorage.setItem("mailcrane-theme", theme);
    } catch (error) {
      // Storage can be blocked (private browsing, strict settings); the theme still applies for this page.
    }
  }
}

applyTheme(document.documentElement.dataset.theme === "dark" ? "dark" : "light");
themeToggle.addEventListener("click", () => {
  applyTheme(document.documentElement.dataset.theme === "dark" ? "light" : "dark", true);
});

const systemColorScheme = window.matchMedia("(prefers-color-scheme: dark)");
let userChoseTheme = false;
try {
  const saved = localStorage.getItem("mailcrane-theme");
  userChoseTheme = saved === "light" || saved === "dark";
} catch (error) {
  // Storage is unavailable, so follow the system setting.
}
themeToggle.addEventListener("click", () => {
  userChoseTheme = true;
});
systemColorScheme.addEventListener("change", (event) => {
  if (!userChoseTheme) {
    applyTheme(event.matches ? "dark" : "light");
  }
});
