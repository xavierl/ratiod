export type Status = 'starting' | 'seeding';

export interface AppConfig {
  minUploadRate: number;
  maxUploadRate: number;
  simultaneousSeed: number;
  client: string;
  keepTorrentWithZeroLeechers: boolean;
  uploadRatioTarget: number;
}

export interface TorrentView {
  infoHash: string;
  name: string;
  size: number;
  status: Status;
  speed: number;
  uploaded: number;
  seeders: number | null;
  leechers: number | null;
  nextAnnounceIn: number | null;
  consecutiveFails: number;
  lastError: string | null;
}

export interface Snapshot {
  running: boolean;
  error: string | null;
  client: string;
  globalSpeed: number;
  sessionUploaded: number;
  queued: number;
  config: AppConfig;
  torrents: TorrentView[];
}

export interface UploadResult {
  file: string;
  status: 'added' | 'duplicate' | 'invalid';
  message?: string;
}

export interface ConfigError {
  field: string;
  message: string;
}
