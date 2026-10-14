import { subscribe } from './api';
import type { Snapshot } from './types';
import type { ToastKind } from './uploads';

export interface Toast {
  id: number;
  kind: ToastKind;
  text: string;
}

class AppState {
  snapshot = $state<Snapshot | null>(null);
  connected = $state(false);
  settingsOpen = $state(false);
  toasts = $state<Toast[]>([]);
  #nextToast = 1;

  connect(): () => void {
    return subscribe(
      (snapshot) => {
        this.snapshot = snapshot;
      },
      (connected) => {
        this.connected = connected;
      },
    );
  }

  notify(kind: ToastKind, text: string): void {
    const id = this.#nextToast++;
    this.toasts.push({ id, kind, text });
    setTimeout(() => this.dismiss(id), kind === 'error' ? 8000 : 5000);
  }

  dismiss(id: number): void {
    this.toasts = this.toasts.filter((t) => t.id !== id);
  }
}

export const app = new AppState();
