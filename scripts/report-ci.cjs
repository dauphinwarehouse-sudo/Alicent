// Runs only in trusted same-repository push/manual workflows, never pull_request_target.
// Publishes job conclusions and bounded GitHub error annotations, not raw logs or secrets.
function overallState(results) {
  const values = Object.values(results);
  if (!values.length) return "error";
  return values.every((value) => value.result === "success")
    ? "success"
    : "failure";
}
function safeText(value, max = 1200) {
  return String(value ?? "")
    .replace(/\u001b\[[0-9;]*m/g, "")
    .replace(/`/g, "'")
    .slice(0, max);
}
async function report({ github, context, core, results }) {
  const { owner, repo } = context.repo;
  const runUrl = `${context.serverUrl}/${owner}/${repo}/actions/runs/${context.runId}`;
  const state = overallState(results);
  const description = Object.entries(results)
    .map(([name, value]) => `${name}: ${value.result}`)
    .join("; ")
    .slice(0, 140);
  await github.rest.repos.createCommitStatus({
    owner,
    repo,
    sha: context.sha,
    context: "alicent/ci",
    state,
    description,
    target_url: runUrl,
  });
  const lines = [
    `<!-- alicent-ci:${context.runId} -->`,
    "## Alicent CI",
    "",
    `Commit: ${context.sha}`,
    `Result: **${state}**`,
    "",
    ...Object.entries(results).map(
      ([name, value]) => `- ${name}: **${value.result}**`,
    ),
    "",
    `[Run and build artifacts](${runUrl})`,
    "",
    "Browser smoke tests are not native Windows E2E. An unsigned installer is not a signed release.",
  ];
  try {
    const jobs = await github.paginate(
      github.rest.actions.listJobsForWorkflowRun,
      { owner, repo, run_id: context.runId, per_page: 100 },
    );
    for (const job of jobs
      .filter((job) => job.conclusion === "failure")
      .slice(0, 6)) {
      lines.push("", `### ${safeText(job.name, 150)}`);
      for (const step of (job.steps ?? []).filter(
        (step) => step.conclusion === "failure",
      ))
        lines.push(`Failed step: ${safeText(step.name, 180)}`);
      const checkRunId = Number(job.check_run_url?.split("/").pop());
      if (Number.isSafeInteger(checkRunId)) {
        const annotations = await github.paginate(
          github.rest.checks.listAnnotations,
          { owner, repo, check_run_id: checkRunId, per_page: 100 },
        );
        for (const annotation of annotations
          .filter((a) => a.annotation_level === "failure")
          .slice(0, 4)) {
          const limit =
            annotation.title === "Rust formatting diff" ? 11000 : 1200;
          lines.push("```text", safeText(annotation.message, limit), "```");
        }
      }
    }
  } catch (error) {
    core.warning(
      `Job annotations unavailable (${error.status ?? "unknown"}). See run link.`,
    );
  }
  await core.summary.addRaw(lines.join("\n")).write();
  const branch = context.ref.replace(/^refs\/heads\//, "");
  const { data: pulls } = await github.rest.pulls.list({
    owner,
    repo,
    state: "open",
    head: `${owner}:${branch}`,
    per_page: 10,
  });
  for (const pr of pulls) {
    await github.rest.issues.createComment({
      owner,
      repo,
      issue_number: pr.number,
      body: lines.join("\n").slice(0, 14000),
    });
  }
}
module.exports = { overallState, safeText, report };
