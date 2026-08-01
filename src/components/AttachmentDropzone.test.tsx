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

function unavailableImageItem(type: string): ClipboardItemFixture {
  return {
    kind: "file",
    type,
    getAsFile: () => null,
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

function LaunchHarness({
  onError,
  onStage,
}: {
  onError?: (message: string) => void;
  onStage?: (entries: LaunchStageEntry[]) => void;
}) {
  const [entries, setEntries] = useState<LaunchStageEntry[]>([]);
  return (
    <AttachmentDropzone
      mode="launch"
      stageEntries={entries}
      onChangeStage={(next) => {
        setEntries(next);
        onStage?.(next);
      }}
      onError={onError}
    />
  );
}

beforeEach(() => {
  vi.mocked(invoke).mockReset();
});

describe("AttachmentDropzone paste behavior", () => {
  it("stages an image exposed only through the async clipboard after WebKitGTK supplies empty paste items", async () => {
    // WebKitGTK bug 218519 can dispatch a real paste gesture with no
    // DataTransfer items even though the clipboard contains image/png.
    // The async Clipboard API remains the only browser-visible byte source.
    const clipboardRead = vi.fn().mockResolvedValue([
      {
        types: ["image/png"],
        getType: vi.fn().mockResolvedValue(new Blob(["png bytes"], { type: "image/png" })),
      },
    ]);
    const previousClipboard = Object.getOwnPropertyDescriptor(navigator, "clipboard");
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { read: clipboardRead },
    });
    try {
      const onStage = vi.fn();
      render(<LaunchHarness onStage={onStage} />);

      paste([]);

      await waitFor(() => expect(clipboardRead).toHaveBeenCalledTimes(1));
      await waitFor(() => expect(onStage).toHaveBeenCalledTimes(1));
      expect(onStage.mock.calls[0][0][0]).toEqual(
        expect.objectContaining({ mime: "image/png", size: 9 }),
      );
    } finally {
      if (previousClipboard) {
        Object.defineProperty(navigator, "clipboard", previousClipboard);
      } else {
        Reflect.deleteProperty(navigator, "clipboard");
      }
    }
  });

  it("shows a soft error when empty WebKitGTK paste items cannot read clipboard bytes", async () => {
    const previousClipboard = Object.getOwnPropertyDescriptor(navigator, "clipboard");
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: undefined });
    try {
      const onError = vi.fn();
      render(<LaunchHarness onError={onError} />);

      const preventDefault = paste([]);

      await waitFor(() => expect(onError).toHaveBeenCalledWith(
        "This webview could not read image bytes from the clipboard. Save it and attach it, or try another clipboard source.",
      ));
      expect(preventDefault).not.toHaveBeenCalled();
    } finally {
      if (previousClipboard) Object.defineProperty(navigator, "clipboard", previousClipboard);
      else Reflect.deleteProperty(navigator, "clipboard");
    }
  });

  it("prevents a single supported image paste and ingests the file", async () => {
    const file = imageFile("screen.png");
    const clipboardRead = vi.fn();
    const previousClipboard = Object.getOwnPropertyDescriptor(navigator, "clipboard");
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { read: clipboardRead },
    });
    vi.mocked(invoke).mockResolvedValue(directAttachment("at-one", file));
    const onAdded = vi.fn();
    try {
      render(<AttachmentDropzone mode="direct" featureId="feature-1" onAdded={onAdded} />);

      const preventDefault = paste([imageItem(file)]);

      expect(preventDefault).toHaveBeenCalledTimes(1);
      await waitFor(() => expect(onAdded).toHaveBeenCalledWith(directAttachment("at-one", file)));
      expect(clipboardRead).not.toHaveBeenCalled();
    } finally {
      if (previousClipboard) Object.defineProperty(navigator, "clipboard", previousClipboard);
      else Reflect.deleteProperty(navigator, "clipboard");
    }
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
    const onStage = vi.fn();
    render(<LaunchHarness onStage={onStage} />);

    paste([imageItem(first)]);
    await screen.findByText("first.png");
    expect(onStage).toHaveBeenLastCalledWith([
      expect.objectContaining({
        name: "first.png",
        source_filename: "first.png",
        mime: "image/png",
        size: first.size,
        file: first,
        sourcePath: null,
        previewUrl: expect.stringMatching(/^data:image\/png;base64,/),
      }),
    ]);
    paste([imageItem(duplicate)]);

    await waitFor(() => {
      expect(screen.queryByText("first.png")).not.toBeInTheDocument();
      expect(screen.getByText("duplicate.png")).toBeInTheDocument();
    });
    expect(screen.getAllByRole("button", { name: /^remove /i })).toHaveLength(1);
    expect(invoke).not.toHaveBeenCalled();
  });

  it("reports an unavailable supported clipboard image without consuming paste", () => {
    const onError = vi.fn();
    render(<LaunchHarness onError={onError} />);

    const preventDefault = paste([stringItem("text/plain"), unavailableImageItem("image/png")]);

    expect(preventDefault).not.toHaveBeenCalled();
    expect(onError).toHaveBeenCalledWith(
      "The clipboard offered an image, but this webview could not access its file. Save it and attach it, or try another clipboard source.",
    );
    expect(screen.queryByRole("button", { name: /^remove /i })).not.toBeInTheDocument();
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
