// @vitest-environment jsdom
import { afterEach, describe, expect, it } from "vitest";
import { useState } from "react";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { LoraWeightInput } from "./FineTune";

afterEach(cleanup);

function Harness({ start, seen }: { start: number; seen: number[] }) {
  const [w, setW] = useState(start);
  return (
    <LoraWeightInput
      label="Weight"
      value={w}
      onChange={(v) => {
        seen.push(v);
        setW(v);
      }}
    />
  );
}

describe("add-on weight field", () => {
  it("can be cleared and a new weight typed without snapping back", () => {
    const seen: number[] = [];
    render(<Harness start={0.8} seen={seen} />);
    const input = screen.getByLabelText("Weight") as HTMLInputElement;
    fireEvent.change(input, { target: { value: "" } });
    expect(input.value).toBe("");
    expect(seen).toEqual([]);
    fireEvent.change(input, { target: { value: "1.2" } });
    expect(input.value).toBe("1.2");
    expect(seen).toEqual([1.2]);
  });

  it("shows the stored weight again when left empty", () => {
    const seen: number[] = [];
    render(<Harness start={0.8} seen={seen} />);
    const input = screen.getByLabelText("Weight") as HTMLInputElement;
    fireEvent.change(input, { target: { value: "" } });
    fireEvent.blur(input);
    expect(input.value).toBe("0.8");
  });
});
