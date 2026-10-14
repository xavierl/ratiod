import { describe, expect, it } from 'vitest';
import type { UploadResult } from './types';
import { summarizeUploads } from './uploads';

const added = (file: string): UploadResult => ({ file, status: 'added' });

describe('summarizeUploads', () => {
  it('reports a full success', () => {
    expect(summarizeUploads([added('a'), added('b'), added('c')])).toEqual({ kind: 'success', text: '3 ajoutés' });
  });

  it('warns about duplicates', () => {
    expect(summarizeUploads([added('a'), { file: 'b', status: 'duplicate', message: 'déjà présent' }])).toEqual({
      kind: 'warning',
      text: '1 ajouté, 1 doublon',
    });
  });

  it('names the first invalid file', () => {
    expect(summarizeUploads([{ file: 'bad.torrent', status: 'invalid', message: 'illisible' }])).toEqual({
      kind: 'error',
      text: '1 invalide — bad.torrent : illisible',
    });
  });

  it('handles an empty answer', () => {
    expect(summarizeUploads([])).toEqual({ kind: 'warning', text: 'Aucun fichier envoyé' });
  });
});
