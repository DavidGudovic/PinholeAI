// @vitest-environment jsdom
import { afterEach, describe, expect, it } from "vitest";
import { focusTabField } from "./focus";

const wait = () => new Promise((r) => requestAnimationFrame(() => setTimeout(r, 5)));

afterEach(() => {
  document.body.innerHTML = "";
});

describe("focusTabField", () => {
  it("puts the cursor at the end of the shown tab's field", async () => {
    document.body.innerHTML = `<div id="tab-create"><textarea id="prompt">a fox</textarea></div><button id="b">x</button>`;
    (document.getElementById("b") as HTMLElement).focus();
    focusTabField("create");
    await wait();
    const prompt = document.getElementById("prompt") as HTMLTextAreaElement;
    expect(document.activeElement).toBe(prompt);
    expect(prompt.selectionStart).toBe(5);
  });

  it("leaves focus alone in another text field, in a hidden tab or with a dialog open", async () => {
    document.body.innerHTML = `<div id="tab-create"><textarea id="prompt"></textarea></div><input id="neg" />`;
    const neg = document.getElementById("neg") as HTMLElement;
    neg.focus();
    focusTabField("create");
    await wait();
    expect(document.activeElement).toBe(neg);

    document.body.innerHTML = `<div id="tab-create" hidden><textarea id="prompt"></textarea></div>`;
    focusTabField("create");
    await wait();
    expect(document.activeElement).toBe(document.body);

    document.body.innerHTML = `<div id="tab-create"><textarea id="prompt"></textarea></div><div role="dialog"></div>`;
    focusTabField("create");
    await wait();
    expect(document.activeElement).toBe(document.body);
  });
});
