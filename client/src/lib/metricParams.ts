import type { MetricParam } from './api';

/** Extract distinct `{{name}}` placeholder names from metric SQL (also matches
 *  placeholders inside `[[ ]]` optional blocks). */
export function extractParamNames(sql: string): string[] {
  const names: string[] = [];
  const seen = new Set<string>();
  const re = /\{\{\s*([A-Za-z_][A-Za-z0-9_]*)\s*\}\}/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(sql)) !== null) {
    if (!seen.has(m[1])) {
      seen.add(m[1]);
      names.push(m[1]);
    }
  }
  return names;
}

/** Realign a metric's declared params with the placeholders actually present in
 *  `sql`: drop definitions whose placeholder is gone, keep the existing label /
 *  default / type for the ones that remain, and declare newly added names.
 *  Returns null when the SQL has no placeholders, which is what the API expects
 *  for "this metric takes no parameters". */
export function syncParams(
  sql: string,
  existing: MetricParam[] | null | undefined,
): MetricParam[] | null {
  const byName = new Map((existing ?? []).map((p) => [p.name, p]));
  const params = extractParamNames(sql).map((name) => byName.get(name) ?? { name });
  return params.length ? params : null;
}
