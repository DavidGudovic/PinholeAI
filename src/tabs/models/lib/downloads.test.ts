import { describe, expect, it } from "vitest";
import type { GroupStatus } from "../../../lib/types";
import { getTagged, hideFinished, knownGroupIds, newestGroupSince, tagGroup, upsertGroup } from "./downloads";

const g = (groupId: string, label: string, state: GroupStatus["state"] = "downloading"): GroupStatus => ({
  groupId,
  label,
  state,
  currentFile: null,
  fileIndex: 0,
  fileCount: 1,
  downloadedBytes: 0,
  totalBytes: 100,
  error: null,
});

describe("downloads store", () => {
  it("keeps the latest status per group and resolves tags", () => {
    upsertGroup(g("a", "Z-Image Turbo"));
    tagGroup("rec:realistic", "a");
    upsertGroup({ ...g("a", "Z-Image Turbo"), downloadedBytes: 50 });
    expect(getTagged("rec:realistic")?.downloadedBytes).toBe(50);
    expect(getTagged("rec:missing")).toBeNull();
  });

  it("finds the group a call started without returning its id (engine)", () => {
    const before = knownGroupIds();
    upsertGroup(g("x1", "Some model"));
    upsertGroup(g("x2", "Image engine (CUDA)"));
    upsertGroup(g("x3", "Another model"));
    expect(newestGroupSince(before, /engine/i)?.groupId).toBe("x2");
    expect(newestGroupSince(before)?.groupId).toBe("x3");
    expect(newestGroupSince(knownGroupIds(), /engine/i)).toBeNull();
  });

  it("can hide finished groups without touching active ones", () => {
    upsertGroup(g("done1", "Finished", "done"));
    hideFinished();
    // Tags still resolve after hiding (hiding only affects the list view).
    tagGroup("t", "done1");
    expect(getTagged("t")?.state).toBe("done");
  });
});
