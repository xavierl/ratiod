import { translate, translateCode, translateCount } from './i18n/core';
import type { Locale } from './i18n/core';
import type { UploadResult } from './types';

export type ToastKind = 'success' | 'warning' | 'error';

/** One toast for a whole batch of uploaded files. */
export function summarizeUploads(results: UploadResult[], locale: Locale): { kind: ToastKind; text: string } {
  if (results.length === 0) return { kind: 'warning', text: translate(locale, 'upload.none') };
  const added = results.filter((r) => r.status === 'added').length;
  const duplicates = results.filter((r) => r.status === 'duplicate').length;
  const invalid = results.filter((r) => r.status === 'invalid');
  const parts: string[] = [];
  if (added) parts.push(translateCount(locale, 'upload.added', added));
  if (duplicates) parts.push(translateCount(locale, 'upload.duplicate', duplicates));
  if (invalid.length) parts.push(translateCount(locale, 'upload.invalid', invalid.length));
  let text = parts.join(', ');
  if (invalid.length) {
    const first = invalid[0];
    const fallback = first.message ?? translate(locale, 'upload.rejected');
    text = translate(locale, 'upload.firstInvalid', {
      summary: text,
      file: first.file,
      reason: translateCode(locale, 'uploadCode', first.code, fallback),
    });
  }
  let kind: ToastKind;
  if (invalid.length === 0) kind = added === results.length ? 'success' : 'warning';
  else kind = added > 0 ? 'warning' : 'error';
  return { kind, text };
}
