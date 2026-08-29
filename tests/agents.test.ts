import assert from "node:assert/strict";
import { test } from "node:test";
import { preferredAgentsFile } from "../src/app/agents.ts";
import { invokeDemo } from "../src/app/demo.ts";
import { COMMANDS, type AgentsChain, type ProjectSummary } from "../src/shared/contracts.ts";

const project: ProjectSummary = {
  id: "project-1",
  name: "fixture",
  canonicalPath: "/projects/fixture",
  source: "observed",
  exists: true,
  isGit: true,
  worktree: false,
  lastSeenAt: null,
  lastConversationAt: null,
  agentsFileCount: 1,
  hasAgentsFile: true,
};

function chainWith(files: AgentsChain["files"]): AgentsChain {
  return { project, selectedCwd: project.canonicalPath, files, effectivePaths: [], warnings: [], maxBytes: 32_768 };
}

function file(path: string, kind: AgentsChain["files"][number]["kind"]) {
  return {
    path,
    relativePath: path,
    kind,
    precedence: 0,
    effective: false,
    overridden: false,
    sha256: "fixture",
    mtimeMs: 0,
    sizeBytes: 0,
    writable: false,
  };
}

test("优先打开顶级全局 AGENTS.md，即使它只读或被覆盖", () => {
  const global = file("/Users/fixture/.codex/AGENTS.md", "global");
  const selected = preferredAgentsFile(chainWith([
    file("/projects/fixture/AGENTS.md", "project"),
    global,
  ]));

  assert.equal(selected?.path, global.path);
});

test("缺少顶级全局 AGENTS.md 时回退到其他全局或项目文件", () => {
  const fallbackGlobal = file("/Users/fixture/.codex/AGENTS.override.md", "global");
  assert.equal(preferredAgentsFile(chainWith([file("/projects/fixture/AGENTS.md", "project"), fallbackGlobal]))?.path, fallbackGlobal.path);
  assert.equal(preferredAgentsFile(chainWith([file("/projects/fixture/AGENTS.md", "project")]))?.path, "/projects/fixture/AGENTS.md");
});

test("创建项目级 AGENTS.md 后项目列表与层级状态同步", async () => {
  const projectPath = "/Users/example/Projects/marketing-site";
  await invokeDemo(COMMANDS.createAgentsFile, {
    input: {
      projectPath,
      authorizedRoot: "/Users/example/Projects",
      content: "# fixture\n",
      fileName: "AGENTS.md",
    },
  });

  const [projects, chain] = await Promise.all([
    invokeDemo<ProjectSummary[]>(COMMANDS.listProjects),
    invokeDemo<AgentsChain>(COMMANDS.getAgentsChain, { projectPath, selectedCwd: projectPath }),
  ]);
  const updated = projects.find((candidate) => candidate.canonicalPath === projectPath);

  assert.equal(updated?.hasAgentsFile, true);
  assert.equal(updated?.agentsFileCount, 1);
  assert.equal(chain.project.hasAgentsFile, true);
  assert.ok(chain.files.some((candidate) => candidate.path === `${projectPath}/AGENTS.md`));
});
