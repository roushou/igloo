// Generates src/api/schema.gen.ts from ../schemas/openapi.json. `--check` fails when the
// committed file is stale.
import { readFile, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import openapiTS, { astToString } from "openapi-typescript";

const source = new URL("../../schemas/openapi.json", import.meta.url);
const target = new URL("../src/api/schema.gen.ts", import.meta.url);
const header = "// Generated from schemas/openapi.json by `bun run gen:api`. Do not edit.\n";

const generated = header + astToString(await openapiTS(source));

if (process.argv.includes("--check")) {
  const current = await readFile(target, "utf8").catch(() => "");
  if (current !== generated) {
    console.error(
      `${fileURLToPath(target)} is stale: run \`bun run gen:api\` in web/ and commit the result.`,
    );
    process.exit(1);
  }
} else {
  await writeFile(target, generated);
}
