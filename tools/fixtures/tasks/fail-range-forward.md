<!-- expect: S3: needs S1-S3 names a slice that is not lower-numbered -->
## Work package 0: A

- [ ] **S1** One. Text.
  needs: none | proof: `true`
- [ ] **S2** Two. Text.
  needs: S1 | proof: `true`
- [ ] **S3** Three. Text.
  needs: S1-S3 | proof: `true`
