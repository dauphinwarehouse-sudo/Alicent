const { test } = require("node:test");
const assert = require("node:assert/strict");
const { overallState, safeText, report } = require("./report-ci.cjs");
test("only all-success is success; skipped and cancelled cannot pass", () => {
  assert.equal(
    overallState({
      core: { result: "success" },
      windows: { result: "success" },
    }),
    "success",
  );
  for (const result of ["failure", "cancelled", "skipped"])
    assert.equal(
      overallState({ core: { result: "success" }, windows: { result } }),
      "failure",
    );
  assert.equal(overallState({}), "error");
});
test("annotation text is bounded and cannot close a markdown fence", () => {
  assert.equal(safeText("\u001b[31m```secretless", 6), "'''sec");
});
test("status report pins the real commit and posts scoped PR results", async () => {
  const calls = [];
  const github = {
    rest: {
      repos: {
        createCommitStatus: async (input) => calls.push(["status", input]),
      },
      actions: { listJobsForWorkflowRun: () => {} },
      checks: { listAnnotations: () => {} },
      pulls: { list: async () => ({ data: [{ number: 1 }] }) },
      issues: {
        createComment: async (input) => calls.push(["comment", input]),
      },
    },
    paginate: async () => [],
  };
  const core = {
    summary: {
      addRaw() {
        return this;
      },
      async write() {},
    },
    warning() {},
  };
  await report({
    github,
    core,
    context: {
      repo: { owner: "owner", repo: "repo" },
      serverUrl: "https://github.com",
      runId: 1,
      sha: "abc",
      ref: "refs/heads/feat/core",
    },
    results: { core: { result: "success" } },
  });
  assert.equal(calls[0][1].sha, "abc");
  assert.equal(calls[0][1].context, "alicent/ci");
  assert.equal(calls[1][1].issue_number, 1);
  assert.match(calls[1][1].body, /Commit: abc/);
});

test("rustfmt diagnostics are reviewable while other annotations remain bounded", async () => {
  const comments = [];
  const github = {
    rest: {
      repos: { createCommitStatus: async () => {} },
      actions: { listJobsForWorkflowRun() {} },
      checks: { listAnnotations() {} },
      pulls: { list: async () => ({ data: [{ number: 1 }] }) },
      issues: { createComment: async (input) => comments.push(input.body) },
    },
    async paginate(method) {
      if (method === this.rest.actions.listJobsForWorkflowRun)
        return [
          { name: "Rust", conclusion: "failure", check_run_url: "/checks/7" },
        ];
      return [
        {
          annotation_level: "failure",
          title: "Rust formatting diff",
          message: "\u001b[31m```" + "x".repeat(2000) + "end-of-format-diff",
        },
        {
          annotation_level: "failure",
          title: "Other failure",
          message: "y".repeat(1300) + "must-not-appear",
        },
      ];
    },
  };
  const core = {
    summary: {
      addRaw() {
        return this;
      },
      async write() {},
    },
    warning() {},
  };
  await report({
    github,
    core,
    context: {
      repo: { owner: "owner", repo: "repo" },
      serverUrl: "https://github.com",
      runId: 2,
      sha: "exact-head",
      ref: "refs/heads/feat/core",
    },
    results: { core: { result: "failure" } },
  });
  assert.equal(comments.length, 1);
  assert.match(comments[0], /end-of-format-diff/);
  assert.doesNotMatch(comments[0], /must-not-appear/);
  assert.doesNotMatch(comments[0], /\u001b/);
  assert.ok(comments[0].includes("'''"));
  assert.ok(comments[0].length <= 14000);
  assert.match(comments[0], /Result: \*\*failure\*\*/);
});
