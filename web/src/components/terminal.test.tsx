import { act, render, screen, waitFor } from "@testing-library/react";
import { vi } from "vitest";
import { Terminal } from "@/components/terminal";
import { signIn, TOKEN } from "@/test/server";

/** The screen xterm would draw: records what is written and lets the test type and resize. */
const screens: FakeScreen[] = [];

class FakeScreen {
  cols = 100;
  rows = 30;
  options: { disableStdin?: boolean } & Record<string, unknown>;
  written: string[] = [];
  disposed = false;
  focused = false;
  private data: ((data: string) => void)[] = [];
  private binary: ((data: string) => void)[] = [];
  private resizes: ((size: { cols: number; rows: number }) => void)[] = [];

  constructor(options: Record<string, unknown>) {
    this.options = options;
    screens.push(this);
  }
  loadAddon() {}
  open() {}
  focus() {
    this.focused = true;
  }
  write(data: Uint8Array) {
    this.written.push(new TextDecoder().decode(data));
  }
  dispose() {
    this.disposed = true;
  }
  onData(listener: (data: string) => void) {
    this.data.push(listener);
    return { dispose() {} };
  }
  onBinary(listener: (data: string) => void) {
    this.binary.push(listener);
    return { dispose() {} };
  }
  onResize(listener: (size: { cols: number; rows: number }) => void) {
    this.resizes.push(listener);
    return { dispose() {} };
  }
  type(data: string) {
    for (const listener of this.data) listener(data);
  }
  pasteBinary(data: string) {
    for (const listener of this.binary) listener(data);
  }
  resize(cols: number, rows: number) {
    for (const listener of this.resizes) listener({ cols, rows });
  }
}

vi.mock("@xterm/xterm", () => ({ Terminal: FakeScreen }));
vi.mock("@xterm/addon-fit", () => ({
  FitAddon: class {
    fit() {}
  },
}));

const sockets: FakeSocket[] = [];

class FakeSocket extends EventTarget {
  binaryType = "blob";
  sent: (string | Uint8Array)[] = [];
  closed: number | null = null;
  constructor(
    readonly url: URL,
    readonly protocols: string[],
  ) {
    super();
    sockets.push(this);
  }
  send(data: string | Uint8Array) {
    this.sent.push(data);
  }
  close(code: number) {
    this.closed = code;
  }
  opens() {
    this.dispatchEvent(new Event("open"));
  }
  says(data: string | ArrayBuffer) {
    this.dispatchEvent(new MessageEvent("message", { data }));
  }
  drops() {
    this.dispatchEvent(new Event("close"));
  }
}

beforeEach(() => {
  screens.length = 0;
  sockets.length = 0;
  signIn();
  vi.stubGlobal("WebSocket", FakeSocket);
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe() {}
      disconnect() {}
    },
  );
});

afterEach(() => {
  vi.unstubAllGlobals();
});

const SANDBOX = "sbx_00000000000000000000000001";

/** Mounts a terminal and waits for its socket. */
async function mounted(props: Partial<React.ComponentProps<typeof Terminal>> = {}) {
  const view = render(<Terminal sandboxId={SANDBOX} {...props} />);
  await waitFor(() => expect(sockets).toHaveLength(1));
  const socket = sockets[0] as FakeSocket;
  const fake = screens[0] as FakeScreen;
  return { ...view, socket, fake };
}

describe("Terminal", () => {
  it("opens the sandbox's terminal with its screen and the token, on the code surface", async () => {
    const { socket, fake } = await mounted({ command: ["bash", "-l"] });
    expect(socket.url.pathname).toBe(`/v1/sandboxes/${SANDBOX}/terminal`);
    expect(socket.url.protocol).toBe("ws:");
    expect(socket.url.searchParams.getAll("command")).toEqual(["bash", "-l"]);
    expect(socket.url.searchParams.get("cols")).toBe("100");
    expect(socket.url.searchParams.get("rows")).toBe("30");
    expect(socket.protocols).toEqual(["igloo.terminal.v1", `igloo.bearer.${TOKEN}`]);
    expect(socket.binaryType).toBe("arraybuffer");
    expect(screen.getByRole("status")).toHaveTextContent("Connecting…");
    expect(screen.getByLabelText("Terminal").parentElement).toHaveClass("bg-code");

    act(() => socket.opens());
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
    expect(fake.focused).toBe(true);
  });

  it("types into the process, prints its output and follows resizes", async () => {
    const { socket, fake } = await mounted();
    act(() => socket.opens());

    fake.type("ls\r");
    expect(socket.sent).toEqual([new TextEncoder().encode("ls\r")]);
    act(() => socket.says(new TextEncoder().encode("file.txt\r\n").buffer));
    expect(fake.written).toEqual(["file.txt\r\n"]);

    fake.resize(132, 43);
    expect(socket.sent.at(-1)).toBe(JSON.stringify({ type: "resize", cols: 132, rows: 43 }));
  });

  it("shows how the process ended, stops taking input and reports it once", async () => {
    const onExit = vi.fn();
    const { socket, fake } = await mounted({ onExit });
    act(() => socket.opens());
    act(() => socket.says(JSON.stringify({ type: "exit", code: 7 })));

    expect(await screen.findByText("Exited with code 7")).toBeInTheDocument();
    expect(fake.options.disableStdin).toBe(true);
    expect(onExit).toHaveBeenCalledOnce();
    expect(onExit).toHaveBeenCalledWith({ code: 7 });
    act(() => socket.drops());
    expect(screen.getByText("Exited with code 7")).toBeInTheDocument();
  });

  it("says when the worker was lost or the socket dropped", async () => {
    const lost = await mounted();
    act(() => lost.socket.opens());
    act(() => lost.socket.says(JSON.stringify({ type: "exit", failure: "lost" })));
    expect(await screen.findByText("Connection to the worker lost")).toBeInTheDocument();
    lost.unmount();

    sockets.length = 0;
    const dropped = await mounted();
    act(() => dropped.socket.opens());
    act(() => dropped.socket.drops());
    expect(await screen.findByText("Disconnected")).toBeInTheDocument();
  });

  it("says when the terminal could not be opened", async () => {
    const { socket } = await mounted();
    act(() => socket.drops());
    expect(await screen.findByText("Could not open the terminal")).toBeInTheDocument();
  });

  it("ends the process when it unmounts", async () => {
    const { socket, fake, unmount } = await mounted();
    act(() => socket.opens());
    unmount();
    expect(socket.closed).toBe(1000);
    expect(fake.disposed).toBe(true);
  });
});
