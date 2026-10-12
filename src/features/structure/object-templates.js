/** Starting text for a new view, routine, trigger or event. */

const TEMPLATES = Object.freeze({
  view: "CREATE VIEW view_name AS\nSELECT *\nFROM table_name;",
  procedure: "CREATE PROCEDURE procedure_name(IN param INT)\nBEGIN\n  SELECT param;\nEND",
  function: "CREATE FUNCTION function_name(param INT)\nRETURNS INT\nDETERMINISTIC\nBEGIN\n  RETURN param;\nEND",
  trigger: "CREATE TRIGGER trigger_name\nBEFORE INSERT ON table_name\nFOR EACH ROW\nBEGIN\n  SET NEW.column_name = NEW.column_name;\nEND",
  event: "CREATE EVENT event_name\nON SCHEDULE EVERY 1 DAY\nDO\n  SELECT 1",
});

export const OBJECT_KINDS = Object.freeze(Object.keys(TEMPLATES));

export function templateFor(kind) {
  const template = TEMPLATES[kind];
  if (template === undefined) throw new Error(`No template for ${kind}`);
  return template;
}

/** A tab name that is not taken, like "New procedure" then "New procedure 2". */
export function newObjectName(kind, takenNames) {
  const base = `New ${kind}`;
  if (!takenNames.includes(base)) return base;
  let counter = 2;
  while (takenNames.includes(`${base} ${counter}`)) counter += 1;
  return `${base} ${counter}`;
}
