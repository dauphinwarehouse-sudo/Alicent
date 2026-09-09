const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");

const root = path.resolve(__dirname, "../..");
const schema = JSON.parse(
  fs.readFileSync(
    path.join(__dirname, "diagnostics-preview.schema.json"),
    "utf8",
  ),
);
const valid = JSON.parse(
  fs.readFileSync(
    path.join(root, "tests/fixtures/diagnostics-preview.valid.json"),
    "utf8",
  ),
);

function validatePreview(value) {
  assert.equal(value.schemaVersion, schema.properties.schemaVersion.const);
  assert.ok(!Number.isNaN(Date.parse(value.generatedAt)));
  assert.ok(
    schema.properties.scope.properties.mode.enum.includes(value.scope.mode),
  );
  assert.equal(typeof value.scope.installedApplication, "boolean");
  assert.equal(typeof value.scope.nativeBoundaryExercised, "boolean");
  if (value.scope.mode === "browser-smoke") {
    assert.equal(value.scope.installedApplication, false);
    assert.equal(value.scope.nativeBoundaryExercised, false);
  } else {
    assert.equal(value.scope.installedApplication, true);
  }
  assert.ok(value.environment.os);
  assert.ok(value.environment.runtime);
  assert.ok(value.checks.length > 0);
  for (const check of value.checks) {
    assert.match(check.id, /^[a-z0-9-]+$/);
    assert.ok(["passed", "failed", "skipped"].includes(check.status));
    assert.equal(typeof check.details, "string");
  }
  for (const impact of ["critical", "serious", "moderate", "minor"]) {
    assert.ok(Number.isInteger(value.accessibility.summary[impact]));
    assert.ok(value.accessibility.summary[impact] >= 0);
  }
  for (const artifact of value.artifacts) {
    assert.ok(!path.isAbsolute(artifact.path));
    assert.ok(!path.win32.isAbsolute(artifact.path));
    assert.ok(!artifact.path.split(/[\\/]/).includes(".."));
  }
}

test("valid fixture implements the diagnostics preview v1 contract", () => {
  validatePreview(valid);
});

test("browser smoke cannot claim the native boundary", () => {
  const invalid = structuredClone(valid);
  invalid.scope.nativeBoundaryExercised = true;
  assert.throws(() => validatePreview(invalid));
});

test("artifact paths cannot disclose absolute runner paths", () => {
  const invalid = structuredClone(valid);
  invalid.artifacts[0].path = "C:\\Users\\runneradmin\\result.png";
  assert.throws(() => validatePreview(invalid));
});
