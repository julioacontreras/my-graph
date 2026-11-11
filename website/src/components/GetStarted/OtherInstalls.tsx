import { CodeWindow } from "../ui/CodeWindow";
import { Tabs } from "../ui/Tabs";

export function OtherInstalls() {
  return (
    <Tabs
      label="Other ways to install"
      tabs={[
        { id: "rust-cli", label: "Rust CLI", content: <CodeWindow id="rust-cli" filename="terminal" /> },
        {
          id: "rust-embedded",
          label: "Rust embedded",
          content: (
            <div className="space-y-3">
              <CodeWindow id="Cargo-dep" filename="Cargo.toml" />
              <CodeWindow id="embedded" filename="main.rs" />
              <p className="text-sm text-muted">
                The crate is not on crates.io yet, so add it as a git dependency.
              </p>
            </div>
          ),
        },
      ]}
    />
  );
}
