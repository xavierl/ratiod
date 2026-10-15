import { describe, expect, it } from 'vitest';
import type { UploadResult } from './types';
import { summarizeUploads } from './uploads';

const added = (file: string): UploadResult => ({ file, status: 'added' });
const duplicate = (file: string): UploadResult => ({
  file,
  status: 'duplicate',
  code: 'duplicate',
  message: 'this torrent is already present',
});

describe('summarizeUploads (French)', () => {
  it('reports a full success', () => {
    expect(summarizeUploads([added('a'), added('b'), added('c')], 'fr')).toEqual({ kind: 'success', text: '3 ajoutés' });
    expect(summarizeUploads([added('a')], 'fr')).toEqual({ kind: 'success', text: '1 ajouté' });
  });

  it('warns about duplicates', () => {
    expect(summarizeUploads([added('a'), duplicate('b')], 'fr')).toEqual({ kind: 'warning', text: '1 ajouté, 1 doublon' });
  });

  it('names the first invalid file and translates its code', () => {
    expect(
      summarizeUploads([{ file: 'bad.torrent', status: 'invalid', code: 'no_http_tracker', message: 'english' }], 'fr'),
    ).toEqual({ kind: 'error', text: '1 invalide — bad.torrent : aucun tracker HTTP(S) dans ce torrent' });
  });

  it('falls back to the backend message for an unknown code', () => {
    expect(summarizeUploads([{ file: 'x', status: 'invalid', code: 'brand_new', message: 'boom' }], 'fr').text).toBe(
      '1 invalide — x : boom',
    );
    expect(summarizeUploads([{ file: 'x', status: 'invalid' }], 'fr').text).toBe('1 invalide — x : fichier refusé');
  });

  it('handles an empty answer', () => {
    expect(summarizeUploads([], 'fr')).toEqual({ kind: 'warning', text: 'Aucun fichier envoyé' });
  });
});

describe('summarizeUploads (English)', () => {
  it('reports a full success', () => {
    expect(summarizeUploads([added('a'), added('b'), added('c')], 'en')).toEqual({ kind: 'success', text: '3 added' });
  });

  it('pluralizes duplicates', () => {
    expect(summarizeUploads([added('a'), duplicate('b')], 'en')).toEqual({ kind: 'warning', text: '1 added, 1 duplicate' });
    expect(summarizeUploads([duplicate('a'), duplicate('b')], 'en').text).toBe('2 duplicates');
  });

  it('names the first invalid file and translates its code', () => {
    expect(
      summarizeUploads([{ file: 'bad.torrent', status: 'invalid', code: 'invalid_torrent', message: 'x' }], 'en'),
    ).toEqual({ kind: 'error', text: '1 invalid — bad.torrent: not a valid .torrent file' });
  });

  it('handles an empty answer', () => {
    expect(summarizeUploads([], 'en')).toEqual({ kind: 'warning', text: 'No file uploaded' });
  });
});
