<!-- expect: S1.5 of package 0 is not checked -->
<!-- args: --package 0 -->
## Work package 0: A

- [x] **S1** One. Text.
  needs: none | proof: `true`
- [ ] **S1.5** Between. Text.
  needs: S1 | proof: `true`
- [x] **S2** Two. Text.
  needs: S1 | proof: `true`
