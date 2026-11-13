// Every test file: the route-owners lookup (a validate call after the draft settles, B8) is replaced by
// an empty answer, so it never lands in a test's own scripted fetch. A test that wants owners sets them
// with `vi.mocked(useRouteOwners).mockReturnValue({...})`; the hook itself is tested in owners.test.tsx.
import { vi } from "vite-plus/test";

vi.mock("../data/store/owners", () => ({ useRouteOwners: vi.fn(() => ({})) }));
