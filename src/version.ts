import { createRequire } from "module";
const { version } = createRequire(import.meta.url)("../package.json") as { version: string };
export { version };
