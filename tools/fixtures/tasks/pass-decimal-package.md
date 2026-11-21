<!-- args: --package 0.5 -->
## Work package 0: A

- [ ] **S1** One. Text.
  needs: none | proof: `true`

## Work package 0.5: Between

- [x] **S1.1** Inserted. Text.
  needs: S1 | proof: `true`
- [x] **S1.2** Later. Text.
  needs: S1.1 | proof: `true`

## Work package 1: B

- [ ] **S2** Two. Text.
  needs: S1 | proof: `true`
