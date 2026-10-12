import { afterEach, describe, expect, it } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { TomlookLogo } from "./TomlookLogo";

afterEach(cleanup);

describe("Tomlook Focus identity", () => {
  it("uses accessible, accent-colored vectors at both logo sizes without loading a mascot", () => {
    const { container, rerender } = render(<TomlookLogo />);
    const logo = screen.getByRole("img", { name: "Tomlook Focus logo" });
    expect(logo.tagName.toLowerCase()).toBe("svg");
    expect(logo).toHaveAttribute("viewBox", "0 0 64 64");
    expect(logo).toHaveClass("tomlook-logo");
    expect(container.querySelector("img")).toBeNull();
    const paths = Array.from(logo.querySelectorAll("path"));
    expect(paths).toHaveLength(2);
    expect(paths.every((path) => path.getAttribute("stroke") === "currentColor")).toBe(true);
    expect(paths[0]).toHaveAttribute("d", "M22 10H10V22M42 10H54V22M10 42V54H22M54 42V54H42");
    expect(paths[1]).toHaveAttribute("d", "M22 24H42M32 24V44");
    rerender(<TomlookLogo compact />);
    expect(screen.getByRole("img", { name: "Tomlook Focus logo" })).toHaveClass("compact");
  });
});
