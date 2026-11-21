import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";
import { Button } from "./Button";
import { Card } from "./Card";
import { Disclosure } from "./Disclosure";
import { Modal } from "./Modal";
import { Money } from "./Money";
import { EmptyView, ErrorView, LoadingView } from "./StateViews";
import { Tag } from "./Tag";

describe("simple components", () => {
  it("Card is a named region", () => {
    render(
      <Card labelledBy="h">
        <h3 id="h">Idea</h3>
      </Card>,
    );
    expect(screen.getByRole("region", { name: "Idea" })).toBeInTheDocument();
  });
  it("Tag shows text and tone class", () => {
    render(<Tag tone="buy">Buy</Tag>);
    expect(screen.getByText("Buy")).toHaveClass("ui-tag-buy");
  });
  it("Button defaults to type=button and fires click", async () => {
    const fn = vi.fn();
    render(<Button onClick={fn}>Go</Button>);
    await userEvent.click(screen.getByRole("button", { name: "Go" }));
    expect(fn).toHaveBeenCalled();
    expect(screen.getByRole("button")).toHaveAttribute("type", "button");
  });
  it("Money groups rupees in en-IN", () => {
    render(<Money value="1000000" />);
    expect(screen.getByText("₹10,00,000.00")).toBeInTheDocument();
    render(<Money value="2500.5" decimals={0} />);
    expect(screen.getByText("₹2,500")).toBeInTheDocument();
  });
});

describe("Disclosure", () => {
  it("toggles aria-expanded and the body", async () => {
    render(<Disclosure summary="Why this idea?">Because of trend.</Disclosure>);
    const b = screen.getByRole("button", { name: "Why this idea?" });
    expect(b).toHaveAttribute("aria-expanded", "false");
    expect(screen.getByText("Because of trend.")).not.toBeVisible();
    await userEvent.click(b);
    expect(b).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByText("Because of trend.")).toBeVisible();
  });
});

describe("StateViews", () => {
  it("loading is a status", () => {
    render(<LoadingView />);
    expect(screen.getByRole("status")).toHaveTextContent("Loading");
  });
  it("empty shows title", () => {
    render(<EmptyView title="No ideas today">Check back later.</EmptyView>);
    expect(screen.getByText("No ideas today")).toBeInTheDocument();
  });
  it("error is an alert with retry", async () => {
    const retry = vi.fn();
    render(<ErrorView message="Could not load." onRetry={retry} />);
    expect(screen.getByRole("alert")).toHaveTextContent("Could not load.");
    await userEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(retry).toHaveBeenCalled();
  });
  it("error without retry has no button", () => {
    render(<ErrorView message="x" />);
    expect(screen.queryByRole("button")).toBeNull();
  });
  it("copy avoids certainty words", () => {
    const { container } = render(
      <>
        <LoadingView />
        <ErrorView message="Could not load." onRetry={() => {}} />
      </>,
    );
    expect(container.textContent).not.toMatch(/\b(will|guaranteed|sure)\b/i);
  });
});

function Harness({ onClose = () => {} }: { onClose?: () => void }) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <button onClick={() => setOpen(true)}>Open</button>
      {open && (
        <Modal
          title="Review paper order"
          onClose={() => {
            onClose();
            setOpen(false);
          }}
        >
          <button>Not now</button>
          <button>Approve paper order</button>
        </Modal>
      )}
    </>
  );
}

describe("Modal", () => {
  it("is labelled, modal, and focuses first control", async () => {
    render(<Harness />);
    await userEvent.click(screen.getByText("Open"));
    const d = screen.getByRole("dialog", { name: "Review paper order" });
    expect(d).toHaveAttribute("aria-modal", "true");
    expect(screen.getByText("Not now")).toHaveFocus();
  });
  it("traps Tab both directions", async () => {
    render(<Harness />);
    await userEvent.click(screen.getByText("Open"));
    await userEvent.tab();
    expect(screen.getByText("Approve paper order")).toHaveFocus();
    await userEvent.tab();
    expect(screen.getByText("Not now")).toHaveFocus();
    await userEvent.tab({ shift: true });
    expect(screen.getByText("Approve paper order")).toHaveFocus();
  });
  it("Esc closes and focus returns to opener", async () => {
    const onClose = vi.fn();
    render(<Harness onClose={onClose} />);
    await userEvent.click(screen.getByText("Open"));
    await userEvent.keyboard("{Escape}");
    expect(onClose).toHaveBeenCalled();
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(screen.getByText("Open")).toHaveFocus();
  });
});
