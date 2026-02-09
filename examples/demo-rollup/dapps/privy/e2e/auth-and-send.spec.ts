import { test, expect } from "@playwright/test";

// Test credentials from environment variables (set from Privy Dashboard)
// For test accounts to work:
// 1. Enable test accounts in Privy Dashboard (User management → Authentication → Advanced)
// 2. Create a test user with a static OTP
// 3. Set PRIVY_TEST_EMAIL and PRIVY_TEST_OTP environment variables
const TEST_EMAIL = process.env.PRIVY_TEST_EMAIL;
const TEST_OTP = process.env.PRIVY_TEST_OTP;

test.describe("Privy Example", () => {
  test("should display connect prompt when not authenticated", async ({
    page,
  }) => {
    await page.goto("/");

    // Wait for Privy to load
    await expect(page.locator("text=Loading Privy...")).toBeHidden({
      timeout: 10000,
    });

    // Should show connect prompt
    await expect(
      page.locator("text=Connect with Privy to send transactions")
    ).toBeVisible();

    // Connect button should be visible
    await expect(page.getByRole("button", { name: "Connect" })).toBeVisible();
  });

  test("should open Privy login modal", async ({ page }) => {
    await page.goto("/");

    // Wait for Privy to load
    await expect(page.locator("text=Loading Privy...")).toBeHidden({
      timeout: 10000,
    });

    // Click Connect button
    await page.getByRole("button", { name: "Connect" }).click();

    // Verify Privy modal opens - check for email input which appears in the modal
    await expect(page.locator('input[type="email"]')).toBeVisible({ timeout: 10000 });
  });

  test.describe("with test account", () => {
    test("should authenticate and send transaction", async ({ page }) => {
      // Fail if credentials are missing - ensures CI doesn't silently pass
      if (!TEST_EMAIL || !TEST_OTP) {
        throw new Error("Privy test credentials not configured - set PRIVY_TEST_EMAIL and PRIVY_TEST_OTP");
      }

      await page.goto("/");

      // Wait for Privy to load
      await expect(page.locator("text=Loading Privy...")).toBeHidden({
        timeout: 10000,
      });

      // Click Connect button
      await page.getByRole("button", { name: "Connect" }).click();

      // Click on email login option if available
      const emailOption = page.getByText("Email", { exact: true });
      if ((await emailOption.count()) > 0) {
        await emailOption.click();
      }

      // Enter test email - try multiple selectors
      const emailInput = page.locator('input[type="email"]').or(page.locator('input[name="email"]'));
      await emailInput.waitFor({ timeout: 10000 });
      await emailInput.fill(TEST_EMAIL!);

      // Click submit button
      await page.getByRole("button", { name: "Submit" }).click();

      // Wait for OTP screen - look for the "Enter confirmation code" heading
      await expect(page.getByRole('heading', { name: /confirmation code/i })).toBeVisible({
        timeout: 15000,
      });

      // Find textboxes within the Privy dialog
      const dialog = page.getByRole('dialog');
      const otpInputs = dialog.getByRole('textbox');

      // Click the first input to focus, then type all digits at once
      await otpInputs.first().click();
      await expect(otpInputs.first()).toBeFocused();

      // Type OTP - Privy auto-advances between inputs
      await page.keyboard.type(TEST_OTP!);
      console.log(`Typed OTP: ${TEST_OTP}`);

      // Wait for authentication to complete - check for transaction section heading
      await expect(
        page.getByRole("heading", { name: "Send Transaction" })
      ).toBeVisible({
        timeout: 30000,
      });

      // Click Sign and Send
      await page.locator("text=Sign and Send").click();

      // Wait for result - either success or error
      const successMessage = page.locator("text=Transaction Submitted Successfully");
      const errorMessage = page.locator(".message.error");

      await expect(successMessage.or(errorMessage)).toBeVisible({
        timeout: 30000,
      });

      // Check if we got an error and fail with details
      if (await errorMessage.isVisible()) {
        const errorText = await page.locator(".message.error pre").textContent();
        throw new Error(`Transaction failed: ${errorText}`);
      }

      // Verify success
      await expect(successMessage).toBeVisible();
    });
  });
});
