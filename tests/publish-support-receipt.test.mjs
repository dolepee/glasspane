import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { supportRoomPaths } from "../scripts/publish-bounty-receipt.mjs";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

test("support receipt publisher targets current room artifacts", () => {
  const paths = supportRoomPaths(repoRoot);
  assert.equal(
    path.relative(repoRoot, paths.webReportPath),
    "web/room/glasspane-support.json",
  );
  for (const filePath of [paths.roomPath, paths.verifiedPath, paths.csvPath, paths.webReportPath]) {
    assert.equal(existsSync(filePath), true, `${path.relative(repoRoot, filePath)} should exist`);
  }
});
