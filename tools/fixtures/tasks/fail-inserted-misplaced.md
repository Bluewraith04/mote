<!-- expect: S1.5 must follow S1 -->
## Work package 0: A

- [x] **S1** One. Text.
  needs: none | proof: `true`
- [x] **S2** Two. Text.
  needs: S1 | proof: `true`
- [x] **S1.5** Between. Text.
  needs: S1 | proof: `true`
