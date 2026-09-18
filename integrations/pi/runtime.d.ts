// Pi supplies these at load, and this repository installs none of them: it
// ships one file, not a package. Declared so the type check reads the
// extension's own logic instead of failing to resolve its imports. The shapes
// are deliberately loose: this gate is here to catch what the extension gets
// wrong about itself, not to restate Pi's API.
/* eslint-disable @typescript-eslint/no-explicit-any */

declare module "@earendil-works/pi-coding-agent" {
  export type ExtensionAPI = any;
}

declare module "@earendil-works/pi-tui" {
  export const Text: any;
}

declare module "typebox" {
  export const Type: any;
}

declare module "node:net" {
  const net: any;
  export default net;
}

declare module "node:os" {
  const os: any;
  export default os;
}

declare module "node:path" {
  const path: any;
  export default path;
}

declare namespace NodeJS {
  type Timeout = ReturnType<typeof setTimeout>;
  type ErrnoException = Error & { code?: string };
}
