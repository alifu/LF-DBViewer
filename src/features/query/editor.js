import { autocompletion, closeBrackets, closeBracketsKeymap, completionKeymap } from "@codemirror/autocomplete";
import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import { MariaSQL, MySQL, sql } from "@codemirror/lang-sql";
import { bracketMatching, indentOnInput, syntaxHighlighting } from "@codemirror/language";
import { Compartment, EditorState, Prec, StateEffect, StateField } from "@codemirror/state";
import { Decoration, EditorView, drawSelection, highlightActiveLine, highlightActiveLineGutter, keymap, lineNumbers } from "@codemirror/view";
import { classHighlighter } from "@lezer/highlight";

const setCurrentStatement = StateEffect.define();

/** The statement the cursor is in, tinted so it is clear what ⌘↵ will run. It follows edits until the next update. */
const currentStatement = StateField.define({
  create: () => Decoration.none,
  update(decorations, transaction) {
    let next = decorations.map(transaction.changes);
    for (const effect of transaction.effects) {
      if (!effect.is(setCurrentStatement)) continue;
      const range = effect.value;
      next = range === null || range.to <= range.from ? Decoration.none : Decoration.set([Decoration.mark({ class: "cm-current-statement" }).range(range.from, range.to)]);
    }
    return next;
  },
  provide: (field) => EditorView.decorations.from(field),
});

const DIALECTS = { mysql: MySQL, mariadb: MariaSQL };

function languageFor(engine, completion) {
  return sql({ dialect: DIALECTS[engine] ?? MySQL, upperCaseKeywords: true, ...completion });
}

/**
 * The SQL editor. Colours come from CSS (`.tok-*` classes and the theme tokens), so it
 * follows light and dark mode without any code here.
 * `onRun(mode)` is called for ⌘↵ ("current") and ⇧⌘↵ ("all"); `onCancel` for ⌘.
 */
export function createSqlEditor({ parent, engine, onRun, onCancel, onChange, onSave = () => {}, onCursor = () => {} }) {
  const language = new Compartment();
  const runKeys = Prec.highest(
    keymap.of([
      { key: "Mod-Enter", run: () => (onRun("current"), true) },
      { key: "Mod-Shift-Enter", run: () => (onRun("all"), true) },
      { key: "Mod-.", run: () => (onCancel(), true) },
      { key: "Mod-s", run: () => (onSave(), true) },
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
        currentStatement,
        keymap.of([...closeBracketsKeymap, ...defaultKeymap, ...historyKeymap, ...completionKeymap, indentWithTab]),
        EditorView.updateListener.of((update) => {
          if (update.docChanged) onChange();
          if (update.docChanged || update.selectionSet) onCursor();
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
    /** Tints `{ from, to }` as the current statement, or clears it with `null`. */
    highlight: (range) => view.dispatch({ effects: setCurrentStatement.of(range) }),
    getSelectionRange: () => ({ from: view.state.selection.main.from, to: view.state.selection.main.to }),
    replaceRange: (from, to, text) => view.dispatch({ changes: { from, to, insert: text } }),
    focus: () => view.focus(),
    destroy: () => view.destroy(),
  };
}
