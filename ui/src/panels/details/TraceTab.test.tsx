import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, test } from "vite-plus/test";
import { useMode } from "../../data/store/mode";
import { TraceTab } from "./TraceTab";
import { useLogDrawer } from "./logDrawer";

afterEach(() => {
  cleanup();
  useLogDrawer.setState({ out: false });
  useMode.setState({ mode: "live" });
});

for (const mode of ["live", "admin"] as const) {
  test(`the_${mode}_trace_pops_out_into_the_drawer_and_comes_back`, async () => {
    useMode.setState({ mode });
    const rail = render(<TraceTab />);
    fireEvent.click(await screen.findByRole("button", { name: /Pop out/ }));
    expect(useLogDrawer.getState().out).toBe(true);
    // the rail says where the list went; the drawer holds the list and has no Pop out of its own
    expect(await screen.findByText(/open along the bottom/)).toBeTruthy();
    expect(screen.queryByRole("button", { name: /Pop out/ })).toBeNull();
    rail.unmount();
    render(<TraceTab drawer />);
    expect(await screen.findByRole("button", { name: /Freeze/ })).toBeTruthy();
    expect(screen.queryByRole("button", { name: /Pop out/ })).toBeNull();
    cleanup();
    render(<TraceTab />);
    fireEvent.click(await screen.findByRole("button", { name: /Bring it back/ }));
    expect(useLogDrawer.getState().out).toBe(false);
  });
}
