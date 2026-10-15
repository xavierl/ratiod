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
  /** English text from the backend; the UI prefers `lastErrorCode` + `lastErrorDetail`. */
  lastError: string | null;
  lastErrorCode: string | null;
  lastErrorDetail: string | null;
}

export interface Snapshot {
  running: boolean;
  /** English text from the backend; the UI prefers `errorCode` + `errorDetail`. */
  error: string | null;
  errorCode: string | null;
  errorDetail: string | null;
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
  /** Stable reason code (absent when added); `message` is its English text. */
  code?: string;
  message?: string;
}

export interface ConfigError {
  field: string;
  code: string;
  message: string;
}
