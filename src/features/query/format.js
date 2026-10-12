import { format } from "sql-formatter";

const INDENT_SPACES = 2;

/** The script laid out readably: one clause per line, keywords in capitals. Throws when the SQL cannot be parsed. */
export function formatSql(text, engine) {
  return format(text, { language: engine === "mariadb" ? "mariadb" : "mysql", keywordCase: "upper", tabWidth: INDENT_SPACES, linesBetweenQueries: 1 });
}
