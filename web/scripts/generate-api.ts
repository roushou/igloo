// Generates src/api/schema.gen.ts from ../schemas/openapi.json. `--check` fails when the
// committed file is stale.
import { readFile, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import openapiTS, { astToString, type OpenAPI3 } from "openapi-typescript";

const source = new URL("../../schemas/openapi.json", import.meta.url);
const target = new URL("../src/api/schema.gen.ts", import.meta.url);
const header = "// Generated from schemas/openapi.json by `bun run gen:api`. Do not edit.\n";

// The client addresses operations by path and method, so `operationId`, which the document
// does not keep unique, is dropped before validation.
const document = JSON.parse(await readFile(source, "utf8")) as OpenAPI3;
for (const item of Object.values(document.paths ?? {})) {
  for (const operation of Object.values(item)) {
    if (operation && typeof operation === "object" && "operationId" in operation) {
      delete operation.operationId;
    }
  }
}

const generated = header + astToString(await openapiTS(document));

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
