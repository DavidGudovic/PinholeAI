import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { CreatorText, hasCreatorText, safeHref } from "./CreatorText";

describe("CreatorText", () => {
  it("keeps formatting and drops scripts, images, iframes and styles", () => {
    const html =
      '<p>Hello <strong>world</strong></p><script>window.hacked=1</script><img src="https://evil.example/t.png"><iframe src="https://evil.example"></iframe><style>p{}</style><ul><li>one</li></ul>';
    const { container } = render(<CreatorText html={html} onLink={() => {}} />);
    expect(screen.getByText("world")).toBeTruthy();
    expect(screen.getByText("one")).toBeTruthy();
    expect(container.querySelector("script, img, iframe, style")).toBeNull();
    expect(container.textContent).not.toContain("hacked");
    expect((window as unknown as { hacked?: number }).hacked).toBeUndefined();
  });

  it("opens https links through onLink and shows other links as text", () => {
    const onLink = vi.fn();
    const { container } = render(<CreatorText html={'<a href="https://example.com/x">good</a> <a href="javascript:alert(1)">bad</a> <a href="http://example.com">plain</a>'} onLink={onLink} />);
    expect(container.querySelectorAll("a")).toHaveLength(1);
    fireEvent.click(screen.getByText("good"));
    expect(onLink).toHaveBeenCalledWith("https://example.com/x");
    fireEvent.keyDown(screen.getByText("good"), { key: "Enter" });
    expect(onLink).toHaveBeenCalledTimes(2);
    expect(screen.getByText(/bad/)).toBeTruthy();
  });

  it("renders links without an href so the WebView never resolves their hosts", () => {
    const { container } = render(<CreatorText html={'<p><a href="https://example.com/x">site</a></p>'} onLink={() => {}} />);
    expect(container.querySelector("[href]")).toBeNull();
    expect(screen.getByRole("link", { name: "site" }).getAttribute("title")).toBe("https://example.com/x");
  });

  it("collapses long text behind Show more", () => {
    const { container } = render(<CreatorText html={`<p>${"word ".repeat(300)}</p>`} onLink={() => {}} />);
    expect(container.querySelector(".max-h-40")).not.toBeNull();
    fireEvent.click(screen.getByText("Show more"));
    expect(screen.getByText("Show less")).toBeTruthy();
    expect(container.querySelector(".max-h-40")).toBeNull();
  });

  it("renders nothing for an empty description", () => {
    const { container } = render(<CreatorText html="<p> </p><img src=x>" onLink={() => {}} />);
    expect(container.textContent).toBe("");
  });

  it("knows when a description is only pictures", () => {
    expect(hasCreatorText('<p><img src="https://x.example/a.png"></p>')).toBe(false);
    expect(hasCreatorText("<p>hi</p>")).toBe(true);
  });

  it("safeHref allows https only", () => {
    expect(safeHref("https://a.b/c")).toBe("https://a.b/c");
    for (const bad of ["http://a.b", "javascript:1", "data:text/html,x", "/relative", "https://u:p@a.b", null]) expect(safeHref(bad)).toBeNull();
  });
});
