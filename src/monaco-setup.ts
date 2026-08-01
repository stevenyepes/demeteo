// Bundle the Monaco editor locally so the UI works fully offline.
//
// By default `@monaco-editor/react` lazy-loads the editor from a CDN
// (https://cdn.jsdelivr.net/npm/monaco-editor/...), which breaks when there is
// no internet. Here we import the `monaco-editor` package that ships in
// node_modules, wire up its web workers through Vite's `?worker` imports, and
// point the react wrapper's loader at the local instance.
import * as monaco from "monaco-editor";
import { loader } from "@monaco-editor/react";

// Worker paths go through the package's `exports` map ("./*" -> "./esm/vs/*.js"),
// which is why they are not spelled `monaco-editor/esm/vs/...`. monaco 0.56 added
// that map, and a path that reaches around it resolves only as long as the
// bundler is lenient about subpath exports — Vite's does not have to be.
import editorWorker from "monaco-editor/editor/editor.worker?worker";
import jsonWorker from "monaco-editor/language/json/json.worker?worker";
import cssWorker from "monaco-editor/language/css/css.worker?worker";
import htmlWorker from "monaco-editor/language/html/html.worker?worker";
import tsWorker from "monaco-editor/language/typescript/ts.worker?worker";

self.MonacoEnvironment = {
  getWorker(_workerId, label) {
    switch (label) {
      case "json":
        return new jsonWorker();
      case "css":
      case "scss":
      case "less":
        return new cssWorker();
      case "html":
      case "handlebars":
      case "razor":
        return new htmlWorker();
      case "typescript":
      case "javascript":
        return new tsWorker();
      default:
        return new editorWorker();
    }
  },
};

// Use the bundled monaco instead of fetching it from a CDN at runtime.
loader.config({ monaco });
