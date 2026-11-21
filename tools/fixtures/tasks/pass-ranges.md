## Work package 0: A

- [x] **S1** One. Text.
  needs: none | proof: `true`
- [x] **S2** Two. Text.
  needs: S1 | proof: `true`
- [x] **S3** Three. Text.
  needs: S1, S2 | proof: `true`
- [x] **S4** Four. Text.
  needs: S1-S3 | proof: `true`
- [ ] **S5** Five. Text.
  needs: S1, S3-S4 | proof: `cargo test --locked -p x -- a b`
