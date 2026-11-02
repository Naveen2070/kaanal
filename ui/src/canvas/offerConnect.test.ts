import { expect, test, vi } from "vite-plus/test";
import { useDraft } from "../data/store/draft";
import { useToasts } from "../shell/toast/toasts";
import { offerConnect } from "./offerConnect";

test("the_toast_offers_the_wire_and_its_button_connects_them", () => {
  const connect = vi.fn().mockResolvedValue({ ok: true });
  useDraft.setState({ connect } as never);
  offerConnect("nginx", "service-2");
  const t = useToasts.getState().toasts.at(-1);
  expect(t?.text).toBe("Added service-2");
  expect(t?.action?.label).toBe("Connect nginx to service-2");
  t?.action?.run();
  expect(connect).toHaveBeenCalledWith("nginx", "service-2");
});
