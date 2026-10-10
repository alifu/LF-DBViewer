import { autocompletion, closeBrackets, closeBracketsKeymap, completionKeymap } from "@codemirror/autocomplete";
import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import { MariaSQL, MySQL, sql } from "@codemirror/lang-sql";
import { bracketMatching, indentOnInput, syntaxHighlighting } from "@codemirror/language";
import { Compartment, EditorState, Prec } from "@codemirror/state";
import { EditorView, drawSelection, highlightActiveLine, highlightActiveLineGutter, keymap, lineNumbers } from "@codemirror/view";
import { classHighlighter } from "@lezer/highlight";

const DIALECTS = { mysql: MySQL, mariadb: MariaSQL };

function languageFor(engine, completion) {
  return sql({ dialect: DIALECTS[engine] ?? MySQL, upperCaseKeywords: true, ...completion });
}

/**
 * The SQL editor. Colours come from CSS (`.tok-*` classes and the theme tokens), so it
 * follows light and dark mode without any code here.
 * `onRun(mode)` is called for ⌘↵ ("current") and ⇧⌘↵ ("all"); `onCancel` for ⌘.
 */
export function createSqlEditor({ parent, engine, onRun, onCancel, onChange }) {
  const language = new Compartment();
  const runKeys = Prec.highest(
    keymap.of([
      { key: "Mod-Enter", run: () => (onRun("current"), true) },
      { key: "Mod-Shift-Enter", run: () => (onRun("all"), true) },
      { key: "Mod-.", run: () => (onCancel(), true) },
    ]),
  );

  const view = new EditorView({
    parent,
    state: EditorState.create({
      doc: "",
      extensions: [
        runKeys,
        lineNumbers(),
        highlightActiveLineGutter(),
        history(),
        drawSelection(),
        indentOnInput(),
        bracketMatching(),
        closeBrackets(),
        autocompletion(),
        highlightActiveLine(),
        syntaxHighlighting(classHighlighter),
        language.of(languageFor(engine, {})),
        keymap.of([...closeBracketsKeymap, ...defaultKeymap, ...historyKeymap, ...completionKeymap, indentWithTab]),
        EditorView.updateListener.of((update) => {
          if (update.docChanged) onChange();
        }),
      ],
    }),
  });

  return {
    getText: () => view.state.doc.toString(),
    getCursor: () => view.state.selection.main.head,
    /** The selected text, or empty when nothing is selected. */
    getSelectedText() {
      const { from, to } = view.state.selection.main;
      return view.state.sliceDoc(from, to);
    },
    setText(text) {
      view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: text } });
    },
    /** Inserts at the cursor, replacing the selection. */
    insert(text) {
      view.dispatch(view.state.replaceSelection(text));
      view.focus();
    },
    setCompletion(completion) {
      view.dispatch({ effects: language.reconfigure(languageFor(engine, completion)) });
    },
    focus: () => view.focus(),
    destroy: () => view.destroy(),
  };
}
