import type { UploadResult } from './types';

export type ToastKind = 'success' | 'warning' | 'error';

function plural(count: number, word: string): string {
  return `${count} ${word}${count > 1 ? 's' : ''}`;
}

/** One toast for a whole batch of uploaded files. */
export function summarizeUploads(results: UploadResult[]): { kind: ToastKind; text: string } {
  if (results.length === 0) return { kind: 'warning', text: 'Aucun fichier envoyé' };
  const added = results.filter((r) => r.status === 'added').length;
  const duplicates = results.filter((r) => r.status === 'duplicate').length;
  const invalid = results.filter((r) => r.status === 'invalid');
  const parts: string[] = [];
  if (added) parts.push(plural(added, 'ajouté'));
  if (duplicates) parts.push(plural(duplicates, 'doublon'));
  if (invalid.length) parts.push(plural(invalid.length, 'invalide'));
  let text = parts.join(', ');
  if (invalid.length) text += ` — ${invalid[0].file} : ${invalid[0].message ?? 'fichier refusé'}`;
  let kind: ToastKind;
  if (invalid.length === 0) kind = added === results.length ? 'success' : 'warning';
  else kind = added > 0 ? 'warning' : 'error';
  return { kind, text };
}
