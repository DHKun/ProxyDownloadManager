import { describe, expect, it } from "vitest";
import { clientIconKey, normalizeExtension } from "../utils/fileIcon";

describe("normalizeExtension", () => {
  it("lowercases the extension", () => {
    expect(normalizeExtension("Report.PDF")).toBe(".pdf");
    expect(normalizeExtension(".PDF")).toBe(".pdf");
    expect(normalizeExtension("archive.tar.GZ")).toBe(".gz");
  });

  it("uses an empty token when there is no extension", () => {
    expect(normalizeExtension("noext")).toBe("");
    expect(normalizeExtension("file.abcxyz")).toBe(".abcxyz");
  });
});

describe("clientIconKey", () => {
  it("shares a type key across the same extension", () => {
    expect(clientIconKey("a.pdf", "", false)).toBe(clientIconKey("b.PDF", "/tmp/b.pdf", false));
  });

  it("switches to the save path once the file is completed", () => {
    const before = clientIconKey("setup.exe", "C:\\Downloads\\setup.exe", false);
    const after = clientIconKey("setup.exe", "C:\\Downloads\\setup.exe", true);
    expect(before).not.toBe(after);
    expect(after).toBe("file:C:\\Downloads\\setup.exe");
  });

  it("changes when the save path changes", () => {
    expect(clientIconKey("a.zip", "/old/a.zip", true)).not.toBe(clientIconKey("a.zip", "/new/a.zip", true));
  });
});
