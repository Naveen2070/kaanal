// Where a lazy chunk (the Log, Results, charts and command palette) loads. While it loads, the
// fallback holds the final layout. If the import fails, the usual cause is that Studio was updated
// under an open tab and the old chunk file is gone, so it says that and offers a reload; it never
// leaves a blank hole.

import { Component, Suspense, type ReactNode } from "react";

interface Props {
  children: ReactNode;
  fallback: ReactNode;
  /** Replaced in tests; the page reloads by default. */
  onReload?: () => void;
}

export class LazyBoundary extends Component<Props, { failed: boolean }> {
  state = { failed: false };

  static getDerivedStateFromError() {
    return { failed: true };
  }

  render() {
    if (this.state.failed) {
      return (
        <div role="alert" className="p-3 text-sm">
          <p className="mb-2">Studio was updated. Reload to get the new version.</p>
          <button
            type="button"
            onClick={this.props.onReload ?? (() => location.reload())}
            className="rounded border border-control px-2 py-0.5"
          >
            Reload
          </button>
        </div>
      );
    }
    return <Suspense fallback={this.props.fallback}>{this.props.children}</Suspense>;
  }
}
