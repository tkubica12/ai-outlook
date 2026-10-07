import { expect, test } from "@playwright/test";

// The guarantee under test is that the UI never invents calendar data. That has
// to hold whether or not this machine's workspace is connected, so the spec
// asserts the onboarding path when it is not and the no-synthetic-data path in
// both cases.
test("never shows synthetic calendar data, connected or not", async ({ page }) => {
  await page.goto("/");
  await page.waitForSelector(".connection-setup, .week-view, .month-grid, .calendar-empty", {
    timeout: 30_000,
  });

  if (await page.locator(".connection-setup").count()) {
    await expect(page.getByRole("heading", { name: "Connect your Microsoft 365 workspace" })).toBeVisible();
    await expect(page.getByText("Live data only")).toBeVisible();
    await expect(page.getByText(/no longer generates demonstration records/i)).toBeVisible();
    await expect(page.getByRole("button", { name: "Check connection again" })).toBeVisible();
  }

  await expect(page.getByText("Contoso cloud adoption review")).toHaveCount(0);
  await expect(page.getByText("Demonstration workspace")).toHaveCount(0);
});
