import { invoke } from "@tauri-apps/api/core";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useState } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  AttachmentDropzone,
  type LaunchStageEntry,
} from "./AttachmentDropzone";

vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: () => ({
    onDragDropEvent: vi.fn().mockResolvedValue(() => {}),
  }),
}));

interface ClipboardItemFixture {
  kind: string;
  type: string;
  getAsFile: () => File | null;
}

function clipboardData(items: ClipboardItemFixture[]): DataTransfer {
  return { items } as unknown as DataTransfer;
}

function imageItem(file: File): ClipboardItemFixture {
  return {
    kind: "file",
    type: file.type,
    getAsFile: () => file,
  };
}

function stringItem(type: "text/plain" | "text/html"): ClipboardItemFixture {
  return {
    kind: "string",
    type,
    getAsFile: () => null,
  };
}

function imageFile(
  name: string,
  type: string = "image/png",
  bytes: string = "image bytes",
): File {
  return new File([bytes], name, { type });
}

function directAttachment(id: string, file: File) {
  return {
    id,
    name: file.name,
    mime: file.type,
    sha256: id.padEnd(64, "0").slice(0, 64),
    size: file.size,
    source_filename: file.name,
  };
}

function dropzone(): HTMLElement {
  return screen.getByText(/paste an image/i).closest("div[tabindex]") as HTMLElement;
}

function paste(items: ClipboardItemFixture[]) {
  const event = new Event("paste", { bubbles: true, cancelable: true });
  Object.defineProperty(event, "clipboardData", {
    value: clipboardData(items),
  });
  const preventDefault = vi.spyOn(event, "preventDefault");
  fireEvent(dropzone(), event);
  return preventDefault;
}

function LaunchHarness({ onError }: { onError?: (message: string) => void }) {
  const [entries, setEntries] = useState<LaunchStageEntry[]>([]);
  return (
    <AttachmentDropzone
      mode="launch"
      stageEntries={entries}
      onChangeStage={setEntries}
      onError={onError}
    />
  );
}

beforeEach(() => {
  vi.mocked(invoke).mockReset();
});

describe("AttachmentDropzone paste behavior", () => {
  it("prevents a single supported image paste and ingests the file", async () => {
    const file = imageFile("screen.png");
    vi.mocked(invoke).mockResolvedValue(directAttachment("at-one", file));
    const onAdded = vi.fn();
    render(<AttachmentDropzone mode="direct" featureId="feature-1" onAdded={onAdded} />);

    const preventDefault = paste([imageItem(file)]);

    expect(preventDefault).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(onAdded).toHaveBeenCalledWith(directAttachment("at-one", file)));
  });

  it("ingests all supported images from a multi-image paste in order", async () => {
    const png = imageFile("first.png", "image/png", "first");
    const jpeg = imageFile("second.jpg", "image/jpeg", "second");
    vi.mocked(invoke)
      .mockResolvedValueOnce(directAttachment("at-first", png))
      .mockResolvedValueOnce(directAttachment("at-second", jpeg));
    const onAdded = vi.fn();
    render(<AttachmentDropzone mode="direct" featureId="feature-1" onAdded={onAdded} />);

    paste([imageItem(png), imageItem(jpeg)]);

    await waitFor(() => expect(onAdded).toHaveBeenCalledTimes(2));
    expect(onAdded.mock.calls.map(([attachment]) => attachment.source_filename)).toEqual([
      "first.png",
      "second.jpg",
    ]);
  });

  it("accepts and ingests an extensionless supported image file", async () => {
    const file = imageFile("clipboard-image", "image/webp");
    vi.mocked(invoke).mockResolvedValue(directAttachment("at-extensionless", file));
    const onAdded = vi.fn();
    render(<AttachmentDropzone mode="direct" featureId="feature-1" onAdded={onAdded} />);

    paste([imageItem(file)]);

    await waitFor(() => expect(onAdded).toHaveBeenCalledTimes(1));
    expect(invoke).toHaveBeenCalledWith(
      "feature_add_attachment",
      expect.objectContaining({
        featureId: "feature-1",
        mime: "image/webp",
        sourceFilename: "clipboard-image",
      }),
    );
  });

  it.each([
    ["text-only", stringItem("text/plain")],
    ["HTML-only", stringItem("text/html")],
  ])("does not prevent or ingest a %s paste", (_label, item) => {
    render(<AttachmentDropzone mode="direct" featureId="feature-1" />);

    const preventDefault = paste([item]);

    expect(preventDefault).not.toHaveBeenCalled();
    expect(invoke).not.toHaveBeenCalled();
  });

  it.each([
    ["image/bmp", "clipboard.bmp"],
    ["image/svg+xml", "clipboard.svg"],
  ])("does not ingest or prevent unsupported %s images", (type, name) => {
    render(<AttachmentDropzone mode="direct" featureId="feature-1" />);

    const preventDefault = paste([imageItem(imageFile(name, type))]);

    expect(preventDefault).not.toHaveBeenCalled();
    expect(invoke).not.toHaveBeenCalled();
  });

  it("forwards each pasted file through addAttachment in direct mode", async () => {
    const gif = imageFile("first.gif", "image/gif", "gif");
    const webp = imageFile("second.webp", "image/webp", "webp");
    vi.mocked(invoke)
      .mockResolvedValueOnce(directAttachment("at-gif", gif))
      .mockResolvedValueOnce(directAttachment("at-webp", webp));
    render(<AttachmentDropzone mode="direct" featureId="feature-direct" />);

    paste([imageItem(gif), imageItem(webp)]);

    await waitFor(() => expect(invoke).toHaveBeenCalledTimes(2));
    expect(invoke).toHaveBeenNthCalledWith(
      1,
      "feature_add_attachment",
      expect.objectContaining({ featureId: "feature-direct", sourceFilename: "first.gif" }),
    );
    expect(invoke).toHaveBeenNthCalledWith(
      2,
      "feature_add_attachment",
      expect.objectContaining({ featureId: "feature-direct", sourceFilename: "second.webp" }),
    );
  });

  it("computes SHA-256 and dedupes pasted launch files by content hash", async () => {
    const first = imageFile("first.png", "image/png", "identical bytes");
    const duplicate = imageFile("duplicate.png", "image/png", "identical bytes");
    render(<LaunchHarness />);

    paste([imageItem(first)]);
    await screen.findByText("first.png");
    paste([imageItem(duplicate)]);

    await waitFor(() => {
      expect(screen.queryByText("first.png")).not.toBeInTheDocument();
      expect(screen.getByText("duplicate.png")).toBeInTheDocument();
    });
    expect(screen.getAllByRole("button", { name: /^remove /i })).toHaveLength(1);
    expect(invoke).not.toHaveBeenCalled();
  });

  it("delivers a downstream ingest rejection through onError", async () => {
    const file = imageFile("accepted.png");
    vi.mocked(invoke).mockRejectedValue(new Error("backend rejected attachment"));
    const onError = vi.fn();
    render(
      <AttachmentDropzone
        mode="direct"
        featureId="feature-1"
        onError={onError}
      />,
    );

    const preventDefault = paste([imageItem(file)]);

    expect(preventDefault).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(onError).toHaveBeenCalledWith("backend rejected attachment"));
  });
});
