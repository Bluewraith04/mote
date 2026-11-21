<!-- expect: S2 of package 0 is not checked -->
<!-- args: --package 0 -->
## Work package 0: A

- [x] **S1** One. Text.
  needs: none | proof: `true`
- [ ] **S2** Two. Text.
  needs: S1 | proof: `true`
