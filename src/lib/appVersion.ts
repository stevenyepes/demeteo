import { invoke } from '@tauri-apps/api/core';
import type { AppVersion } from '../types';

/** Fetch the build-time application version + release channel. */
export async function getAppVersion(): Promise<AppVersion> {
  return invoke<AppVersion>('get_app_version');
}
