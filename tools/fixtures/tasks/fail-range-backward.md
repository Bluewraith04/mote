<!-- expect: range S2-S1 runs backward -->
## Work package 0: A

- [ ] **S1** One. Text.
  needs: none | proof: `true`
- [ ] **S2** Two. Text.
  needs: S1 | proof: `true`
- [ ] **S3** Three. Text.
  needs: S2-S1 | proof: `true`
