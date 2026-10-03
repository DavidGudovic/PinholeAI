import { describe, expect, it } from "vitest";
import { choiceCount, expandChoices, MAX_CHOICE_PICTURES } from "./choices";

describe("choices in braces", () => {
  it("makes one prompt per choice", () => {
    expect(expandChoices("a {red|blue|green} car").prompts).toEqual(["a red car", "a blue car", "a green car"]);
  });

  it("multiplies several groups, the first changing slowest", () => {
    const c = expandChoices("{small|big} {cat|dog}");
    expect(c.prompts).toEqual(["small cat", "small dog", "big cat", "big dog"]);
    expect(c.total).toBe(4);
  });

  it("leaves braces without a | alone, and prompts without choices unchanged", () => {
    expect(expandChoices("a {cozy} cabin")).toEqual({ prompts: ["a {cozy} cabin"], total: 1 });
    expect(choiceCount("a {cozy} cabin")).toBeNull();
    expect(choiceCount("a cabin")).toBeNull();
  });

  it("trims options, counts repeats once and tidies spaces an empty option leaves", () => {
    expect(expandChoices("a { red | red |blue } car").prompts).toEqual(["a red car", "a blue car"]);
    expect(expandChoices("a {|very} old house, {|at night}").prompts).toEqual([
      "a old house,",
      "a old house, at night",
      "a very old house,",
      "a very old house, at night",
    ]);
    expect(expandChoices("a fox {in snow|}, watercolor").prompts).toEqual(["a fox in snow, watercolor", "a fox, watercolor"]);
  });

  it("never makes an empty prompt, and counts only different prompts", () => {
    expect(expandChoices("{cat|}").prompts).toEqual(["cat"]);
    expect(expandChoices("{|}").prompts).toEqual([]);
    expect(choiceCount("a {big  dog|big dog}")).toBeNull();
    expect(choiceCount("{a|}{|a}")).toEqual({ count: 2, total: 2 });
  });

  it("stops at the cap and reports how many there would be", () => {
    const c = expandChoices("{a|b|c|d|e} {1|2|3|4|5|6}");
    expect(c.prompts).toHaveLength(MAX_CHOICE_PICTURES);
    expect(c.total).toBe(30);
    expect(choiceCount("{a|b|c|d|e} {1|2|3|4|5|6}")).toEqual({ count: 16, total: 30 });
  });

  it("stays quick with a huge number of combinations", () => {
    const prompt = Array.from({ length: 40 }, () => "{x|}").join("");
    const c = expandChoices(prompt);
    expect(c.prompts.length).toBeLessThanOrEqual(MAX_CHOICE_PICTURES);
    expect(c.total).toBe(2 ** 40);
  });
});
