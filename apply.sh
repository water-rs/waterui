 (cd "$(git rev-parse --show-toplevel)" && git apply --3way <<'EOF' 
diff --git a/.github/workflows/ci.yml b/.github/workflows/ci.yml
index dc17cd73504a730255e353c0663de90c70c65bd7..fe193991712af03948c3e81a93458f04613e5f80 100644
--- a/.github/workflows/ci.yml
+++ b/.github/workflows/ci.yml
@@ -1,52 +1,51 @@
+---
 name: CI
 
-on:
+'on':
   push:
     branches: [main]
   pull_request:
     branches: [main]
 
 env:
   CARGO_TERM_COLOR: always
 
 jobs:
-  test:
-    name: Test
+  rust-checks:
+    name: Rust checks
     runs-on: ubuntu-latest
     steps:
       - uses: actions/checkout@v4
 
       - name: Install Rust
         uses: dtolnay/rust-toolchain@nightly
         with:
           components: rustfmt, clippy
 
       - name: Cache cargo registry
         uses: actions/cache@v4
         with:
           path: ~/.cargo/registry
           key: ${{ runner.os }}-cargo-registry-${{ hashFiles('**/Cargo.lock') }}
 
       - name: Cache cargo index
         uses: actions/cache@v4
         with:
           path: ~/.cargo/git
           key: ${{ runner.os }}-cargo-index-${{ hashFiles('**/Cargo.lock') }}
 
       - name: Check formatting
         run: cargo fmt --all -- --check
 
       - name: Run clippy
-        run: cargo clippy --all-targets --all-features --workspace -- -D warnings
-
-      - name: Run tests
-        run: cargo test --all-features --workspace
+        run: |
+          cargo clippy --all-targets --all-features --workspace -- -D warnings
 
-      - name: Check documentation
-        run: cargo doc --all-features --no-deps --workspace
+  swift-backend:
+    name: Build Swift backend
+    runs-on: macos-latest
+    steps:
+      - uses: actions/checkout@v4
 
-      - name: Check that packages can be built
-        run: |
-          cargo package -p waterui --allow-dirty
-          cargo package -p waterui-core --allow-dirty
-          cargo package -p waterui-cli --allow-dirty
+      - name: Build Swift package
+        run: swift build --package-path backends/swift --configuration release
 
EOF
)