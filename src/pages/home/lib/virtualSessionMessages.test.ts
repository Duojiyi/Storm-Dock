import { describe, expect, it } from "vitest";
import {
  SESSION_MESSAGE_ESTIMATE_PX,
  SESSION_MESSAGE_OVERSCAN,
} from "../components/VirtualSessionMessages";

/** Upper bound on mounted rows for a given viewport — mirrors tanstack overscan math. */
export function maxMountedVirtualRows(options: {
  total: number;
  viewportHeight: number;
  estimateSize?: number;
  overscan?: number;
}): number {
  const estimateSize = options.estimateSize ?? SESSION_MESSAGE_ESTIMATE_PX;
  const overscan = options.overscan ?? SESSION_MESSAGE_OVERSCAN;
  if (options.total <= 0 || options.viewportHeight <= 0) return 0;
  const visible = Math.ceil(options.viewportHeight / estimateSize);
  return Math.min(options.total, visible + overscan * 2);
}

describe("virtual session message bounds", () => {
  it("keeps DOM row count bounded for a 2000-message session", () => {
    const mounted = maxMountedVirtualRows({
      total: 2000,
      viewportHeight: 900,
    });
    // ~7 visible + 16 overscan ≈ 23; allow headroom for short estimates
    expect(mounted).toBeLessThanOrEqual(40);
    expect(mounted).toBeLessThan(2000);
  });

  it("renders every row for short sessions", () => {
    expect(
      maxMountedVirtualRows({
        total: 5,
        viewportHeight: 900,
      }),
    ).toBe(5);
  });
});
