import { invoke } from "@tauri-apps/api/core";
import type { Notification } from "../types";

export async function listNotifications(
  projectId?: string | null,
): Promise<Notification[]> {
  return invoke<Notification[]>("notifications_list", {
    projectId: projectId ?? null,
  });
}

export async function markNotificationRead(id: string): Promise<void> {
  return invoke<void>("notification_mark_read", { id });
}

export async function unreadNotificationCount(): Promise<number> {
  return invoke<number>("notification_unread_count");
}
