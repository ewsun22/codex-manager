import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App.tsx";
import "./styles/app.css";

const root = document.getElementById("root");

if (!root) {
  throw new Error("应用挂载节点 #root 不存在。");
}

createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
