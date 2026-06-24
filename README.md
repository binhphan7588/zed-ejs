# zed-ejs

Extension cho [Zed editor](https://zed.dev) hỗ trợ file `.ejs` (Embedded JavaScript Templates):

- Syntax highlighting cho HTML bên ngoài và JavaScript bên trong `<% %>`, `<%= %>`, `<%- %>`.
- Bracket matching / auto-close cho tag EJS và các ngoặc thông thường.
- Code folding cho các khối directive.
- Native formatter thông qua Rust LSP, không cần Node/Prettier hay cấu hình external formatter trong Zed settings.

## Cấu trúc

```text
zed-ejs/
├── extension.toml
├── Cargo.toml
├── src/
│   └── lib.rs              # Zed extension entrypoint
├── crates/
│   └── ejs-lsp/
│       └── src/main.rs     # Native LSP formatter
├── languages/
│   └── ejs/
│       ├── config.toml
│       ├── highlights.scm
│       ├── injections.scm
│       ├── brackets.scm
│       └── folds.scm
└── grammars/
```

## Cài extension trong Zed

1. Mở Zed.
2. Mở Command Palette (`Cmd+Shift+P` trên macOS, `Ctrl+Shift+P` trên Linux/Windows).
3. Chọn `zed: install dev extension`.
4. Chọn thư mục `zed-ejs` chứa `extension.toml`.
5. Mở file `.ejs` để kiểm tra syntax highlighting và formatter.

## Format EJS

Formatter được expose qua language server `ejs-lsp`, nên Zed tự gọi khi chạy Format Document hoặc format on save. Không cần thêm cấu hình external formatter trong settings.

Trong lúc dev local, build native formatter trước:

```bash
cargo build --release --manifest-path crates/ejs-lsp/Cargo.toml
mkdir -p ejs-lsp
cp crates/ejs-lsp/target/release/ejs-lsp ejs-lsp/ejs-lsp
```

Trên Windows PowerShell:

```powershell
cargo build --release --manifest-path crates/ejs-lsp/Cargo.toml
New-Item -ItemType Directory -Force -Path ejs-lsp
Copy-Item -Force crates\ejs-lsp\target\release\ejs-lsp.exe ejs-lsp\ejs-lsp.exe
```

Sau đó giữ binary tại đường dẫn bundled mà extension tìm:

```text
ejs-lsp/ejs-lsp.exe
```

Extension sẽ tự tìm binary ở `ejs-lsp/ejs-lsp` hoặc `ejs-lsp/ejs-lsp.exe` khi chạy local.

Khi publish, tạo GitHub Release chứa asset theo platform:

```text
ejs-lsp-windows-x86_64.zip
ejs-lsp-linux-x86_64.zip
ejs-lsp-darwin-x86_64.zip
ejs-lsp-darwin-aarch64.zip
```

Mỗi zip chứa binary `ejs-lsp` hoặc `ejs-lsp.exe` ở root. Extension sẽ tự tải đúng asset theo OS/architecture.

Ví dụ:

```ejs
<div><%if(users.length){%><span><%=user.name??"Guest"%></span><%}else{%><p>No users</p><%}%></div>
```

sẽ được format thành dạng gần như:

```ejs
<div>
  <% if (users.length) { %>
  <span>
    <%= user.name ?? "Guest" %>
  </span>
  <% } else { %>
  <p>
    No users
  </p>
  <% } %>
</div>
```

## Test formatter

```bash
cargo test --manifest-path crates/ejs-lsp/Cargo.toml
```

## Lưu ý

- Formatter hiện là native formatter nhẹ, không phải clone đầy đủ của Prettier.
- HTML được format theo indentation cơ bản.
- JS trong EJS tag được normalize spacing cho các case phổ biến như `if (...) {`, `} else {`, toán tử, dấu phẩy.
- `<%# %>` là comment EJS nên formatter giữ nguyên nội dung.
- Với logic JavaScript phức tạp trong template, vẫn nên tách ra helper `.js` riêng để template dễ đọc hơn.

## License

MIT
