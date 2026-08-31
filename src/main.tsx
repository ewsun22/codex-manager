import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App.tsx";
import { I18nProvider } from "./app/i18n.tsx";
import { readLocalePreference, resolveLocale, setCurrentLocale } from "./app/i18n-core.ts";
import "./styles/app.css";

const root = document.getElementById("root");

if (!root) {
  throw new Error("应用挂载节点 #root 不存在。");
}

setCurrentLocale(resolveLocale(readLocalePreference()));

createRoot(root).render(
  <StrictMode>
    <I18nProvider>
      <App />
    </I18nProvider>
  </StrictMode>,
);
