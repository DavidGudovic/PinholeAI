import { describe, expect, it } from "vitest";
import type { GroupStatus } from "../../../lib/types";
import { getTagged, hideFinished, knownGroupIds, newestActiveOfKind, newestGroupSince, tagGroup, upsertGroup } from "./downloads";

const g = (groupId: string, label: string, state: GroupStatus["state"] = "downloading", kind: GroupStatus["kind"] = "model"): GroupStatus => ({
  groupId,
  label,
  kind,
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

  it("finds the group a call started without returning its id (engine, by kind)", () => {
    const before = knownGroupIds();
    const isEngine = (x: GroupStatus) => x.kind === "engine";
    upsertGroup(g("x1", "Some model"));
    upsertGroup(g("x2", "Image engine (NVIDIA CUDA)", "downloading", "engine"));
    upsertGroup(g("x3", "Another model"));
    // The Describe engine lives in a "captioner" group: never mistaken for the image engine.
    upsertGroup(g("x4", "Describe engine (Vulkan)", "downloading", "captioner"));
    expect(newestGroupSince(before, isEngine)?.groupId).toBe("x2");
    expect(newestGroupSince(before)?.groupId).toBe("x4");
    expect(newestGroupSince(knownGroupIds(), isEngine)).toBeNull();
    expect(newestActiveOfKind("engine")?.groupId).toBe("x2");
    upsertGroup(g("x2", "Image engine (NVIDIA CUDA)", "done", "engine"));
    expect(newestActiveOfKind("engine")).toBeNull();
  });

  it("can hide finished groups without touching active ones", () => {
    upsertGroup(g("done1", "Finished", "done"));
    hideFinished();
    // Tags still resolve after hiding (hiding only affects the list view).
    tagGroup("t", "done1");
    expect(getTagged("t")?.state).toBe("done");
  });
});
