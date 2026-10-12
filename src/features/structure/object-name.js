/**
 * The name a `CREATE VIEW|PROCEDURE|FUNCTION|TRIGGER|EVENT` statement creates, so a saved new object
 * can be reopened under its real name. Returns null when it cannot be told.
 */
const CREATE_HEAD =
  /^\s*create\s+(?:or\s+replace\s+)?(?:algorithm\s*=\s*\w+\s+)?(?:definer\s*=\s*(?:`[^`]*`|[^\s@]+)@(?:`[^`]*`|[^\s]+)\s+)?(?:sql\s+security\s+\w+\s+)?(?:procedure|function|trigger|event|view)\s+(?:if\s+not\s+exists\s+)?/i;
const NAME = /^(?:`((?:[^`]|``)+)`|([A-Za-z0-9_$]+))(?:\s*\.\s*(?:`((?:[^`]|``)+)`|([A-Za-z0-9_$]+)))?/;

export function objectNameFromSql(sql) {
  const head = CREATE_HEAD.exec(sql);
  if (head === null) return null;
  const match = NAME.exec(sql.slice(head[0].length));
  if (match === null) return null;
  // With `database.name` the second part is the object's name.
  return (match[3] ?? match[4] ?? match[1] ?? match[2]).replaceAll("``", "`");
}
