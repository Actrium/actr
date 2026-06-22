import { createHighlighterCore, type HighlighterCore } from "shiki/core";
import { createJavaScriptRegexEngine } from "shiki/engine/javascript";
import toml from "shiki/langs/toml.mjs";
import githubLight from "shiki/themes/github-light.mjs";

let instance: HighlighterCore | null = null;
let loading: Promise<HighlighterCore> | null = null;

export async function highlightToml(code: string): Promise<string> {
  if (!instance) {
    if (!loading) {
      loading = createHighlighterCore({
        themes: [githubLight],
        langs: [toml],
        engine: createJavaScriptRegexEngine(),
      });
    }
    instance = await loading;
  }
  return instance.codeToHtml(code, { lang: "toml", theme: "github-light" });
}
