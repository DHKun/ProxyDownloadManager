import { describe, expect, it } from "vitest";
import type { DownloadStatus } from "../types";
import {
  connectionOptionLabel,
  proxySelectLabel,
  runtimeControlCapabilities,
} from "../utils/runtimeControls";

function caps(
  status: DownloadStatus,
  resumable: boolean | null,
  fileName = "file.bin",
  contentType = "application/octet-stream",
) {
  return runtimeControlCapabilities({
    status,
    resumable,
    file_name: fileName,
    content_type: contentType,
  });
}

describe("runtimeControlCapabilities", () => {
  it("enables every control for an active ranged download and for paused", () => {
    expect(caps("downloading", true)).toEqual({ proxy: true, connections: true, rateLimit: true });
    expect(caps("connecting", null)).toEqual({ proxy: true, connections: true, rateLimit: true });
    expect(caps("paused", false)).toEqual({ proxy: true, connections: true, rateLimit: true });
  });

  it("keeps only the rate limit for an active single-file or HLS download", () => {
    expect(caps("downloading", false)).toEqual({ proxy: false, connections: false, rateLimit: true });
    expect(caps("merging", true, "show.m3u8", "application/vnd.apple.mpegurl")).toEqual({
      proxy: false,
      connections: false,
      rateLimit: true,
    });
  });

  it("hides controls once the download is finished", () => {
    const off = { proxy: false, connections: false, rateLimit: false };
    expect(caps("completed", true)).toEqual(off);
    expect(caps({ failed: "HTTP 403" }, true)).toEqual(off);
  });

  it("allows queued changes only when the engine will read them at start", () => {
    expect(caps("queued", true).connections).toBe(true);
    expect(caps("queued", false).connections).toBe(false);
    expect(caps("queued", false).proxy).toBe(true);
    expect(caps("queued", false, "a.m3u8").connections).toBe(true);
  });
});

describe("detail option labels", () => {
  it("shows the proxy name and connection count without a repeated prefix", () => {
    expect(proxySelectLabel("", "无代理")).toBe("无代理");
    expect(proxySelectLabel("clash", "无代理")).toBe("clash");
    expect(connectionOptionLabel(0, "自动 (32)")).toBe("自动 (32)");
    expect(connectionOptionLabel(8, "自动 (32)")).toBe("8");
    expect(proxySelectLabel("clash", "无代理").startsWith("代理")).toBe(false);
    expect(connectionOptionLabel(4, "自动 (32)").startsWith("连接")).toBe(false);
  });
});
