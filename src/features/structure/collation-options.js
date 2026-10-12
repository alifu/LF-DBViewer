/** Helpers for the charset and collation pickers; `collations` come from the backend's `serverOptions`. */

export function charsetsOf(collations) {
  return [...new Set(collations.map((collation) => collation.charset))].sort();
}

export function collationsFor(collations, charset) {
  return collations.filter((collation) => collation.charset === charset);
}

/** The collation the server marks as default, else the first one of that charset. */
export function defaultCollationFor(collations, charset) {
  const own = collationsFor(collations, charset);
  return (own.find((collation) => collation.isDefault) ?? own[0])?.name ?? null;
}

/** A collation name starts with its character set: `utf8mb4_bin` is `utf8mb4`. */
export function charsetOfCollation(collationName) {
  return collationName.split("_")[0];
}
