import { describe, it, expect } from "vitest";
import { isFailed, getErrorMessage, statusString, statusLabel, statusColor, formatTimestamp, failureText, errorDetail } from "../utils/format";
import { setLanguage } from "../i18n";

describe("isFailed", () => {
  it("returns true for failed status", () => {
    expect(isFailed({ failed: "timeout" })).toBe(true);
  });

  it("returns false for non-failed statuses", () => {
    expect(isFailed("downloading")).toBe(false);
    expect(isFailed("paused")).toBe(false);
    expect(isFailed("completed")).toBe(false);
    expect(isFailed("queued")).toBe(false);
  });
});

describe("getErrorMessage", () => {
  it("extracts message from failed status", () => {
    expect(getErrorMessage({ failed: "connection refused" })).toBe("connection refused");
  });

  it("returns undefined for non-failed statuses", () => {
    expect(getErrorMessage("downloading")).toBeUndefined();
    expect(getErrorMessage("completed")).toBeUndefined();
  });
});

describe("failureText", () => {
  it("reads an HTTP code from the engine message", () => {
    expect(failureText({ failed: "HTTP 403" })).toEqual({ code: 403, message: "HTTP 403" });
    expect(failureText("failed", "HTTP 404 not found").code).toBe(404);
  });

  it("returns the message when no code is present", () => {
    expect(failureText({ failed: "connection reset" })).toEqual({
      code: null,
      message: "connection reset",
    });
  });
});

describe("errorDetail", () => {
  it("drops a message that only repeats the HTTP code", () => {
    expect(errorDetail(403, "HTTP 403")).toBe("");
    expect(errorDetail(404, "HTTP 404 not found")).toBe("not found");
    expect(errorDetail(403, "Forbidden (HTTP 403)")).toBe("Forbidden");
  });

  it("keeps a message that has no code", () => {
    expect(errorDetail(null, "connection reset")).toBe("connection reset");
  });
});

describe("statusLabel", () => {
  it("uses the active language and falls back for unknown values", () => {
    setLanguage("en");
    expect(statusLabel("paused")).toBe("Paused");
    expect(statusLabel({ failed: "HTTP 403" })).toBe("Failed");
    setLanguage("zh");
    expect(statusLabel("downloading")).toBe("下载中");
    setLanguage("en");
  });
});

describe("statusString", () => {
  it("returns string for string statuses", () => {
    expect(statusString("downloading")).toBe("downloading");
    expect(statusString("completed")).toBe("completed");
  });

  it("returns 'failed' for failed status", () => {
    expect(statusString({ failed: "timeout" })).toBe("failed");
  });
});

describe("statusColor", () => {
  it("returns danger for failed", () => {
    expect(statusColor({ failed: "timeout" })).toBe("danger");
  });

  it("returns success for completed", () => {
    expect(statusColor("completed")).toBe("success");
  });

  it("returns attention for paused", () => {
    expect(statusColor("paused")).toBe("attention");
  });

  it("returns accent for downloading", () => {
    expect(statusColor("downloading")).toBe("accent");
  });

  it("returns default for other statuses", () => {
    expect(statusColor("queued")).toBe("default");
  });
});

describe("formatTimestamp", () => {
  it("returns dash for empty string", () => {
    expect(formatTimestamp("")).toBe("—");
  });

  it("formats valid unix timestamp", () => {
    // 2024-01-01 00:00:00 UTC = 1704067200
    const result = formatTimestamp("1704067200");
    expect(result).toContain("2024");
    expect(result).toContain("01");
  });

  it("returns original string for invalid timestamp", () => {
    expect(formatTimestamp("not-a-number")).toBe("not-a-number");
  });

  it("returns original string for zero timestamp", () => {
    expect(formatTimestamp("0")).toBe("0");
  });
});
