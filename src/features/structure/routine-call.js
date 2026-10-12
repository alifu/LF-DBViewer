/** Builds the SQL that runs a stored procedure or function with values typed in a dialog. */

/** A quoted string literal, or NULL. Quotes are doubled and backslashes escaped (default sql_mode). */
export function sqlLiteral(value) {
  if (value === null) return "NULL";
  return `'${value.replaceAll("\\", "\\\\").replaceAll("'", "''")}'`;
}

const quoteIdentifier = (name) => `\`${name.replaceAll("`", "``")}\``;
const qualified = (database, name) => `${quoteIdentifier(database)}.${quoteIdentifier(name)}`;
const variableFor = (parameter) => `@lf_${(parameter.name ?? `p${parameter.position}`).replace(/\W/g, "_")}`;

/** Parameters the user fills in: IN and INOUT. A function has no modes; its parameters are all inputs. */
export function inputParameters(parameters) {
  return parameters.filter((parameter) => parameter.position > 0 && parameter.mode !== "OUT");
}

/**
 * `values` maps a parameter's position to its text, or `null` for SQL NULL.
 * A procedure with OUT or INOUT parameters is followed by a SELECT that shows them.
 */
export function buildCallSql({ kind, database, name, parameters, values }) {
  const inputs = parameters.filter((parameter) => parameter.position > 0);
  const valueOf = (parameter) => sqlLiteral(values[parameter.position] ?? null);

  if (kind === "function") {
    return `SELECT ${qualified(database, name)}(${inputs.map(valueOf).join(", ")}) AS result;`;
  }

  const setups = inputs.filter((parameter) => parameter.mode === "INOUT").map((parameter) => `SET ${variableFor(parameter)} = ${valueOf(parameter)};`);
  const args = inputs.map((parameter) => (parameter.mode === "IN" ? valueOf(parameter) : variableFor(parameter)));
  const outputs = inputs.filter((parameter) => parameter.mode !== "IN");
  const lines = [...setups, `CALL ${qualified(database, name)}(${args.join(", ")});`];
  if (outputs.length > 0) {
    lines.push(`SELECT ${outputs.map((parameter) => `${variableFor(parameter)} AS ${quoteIdentifier(parameter.name ?? `p${parameter.position}`)}`).join(", ")};`);
  }
  return lines.join("\n");
}
