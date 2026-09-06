;; Play USDCx for Stacks testnet. Deployer + Clarinet wallet_1 can mint.
;; 6 decimals. Not the mainnet Circle token.

(define-constant ERR_NOT_OWNER (err u4))
(define-constant ERR_UNAUTHORIZED (err u400))
(define-constant CONTRACT-OWNER tx-sender)
(define-constant PLAY-MINTER 'ST1SJ3DTE5DN7X54YDH5D64R3BCB6A2AG2ZQ8YPD5)

(define-fungible-token usdcx-token)
(define-constant token-decimals u6)
(define-data-var token-name (string-ascii 32) "USDCx")
(define-data-var token-symbol (string-ascii 10) "USDCx")
(define-data-var token-uri (optional (string-utf8 256)) none)

(define-private (can-mint)
  (or (is-eq tx-sender CONTRACT-OWNER) (is-eq tx-sender PLAY-MINTER))
)

(define-public (transfer
  (amount uint)
  (sender principal)
  (recipient principal)
  (memo (optional (buff 34)))
)
  (begin
    (asserts! (or (is-eq tx-sender sender) (is-eq contract-caller sender)) ERR_NOT_OWNER)
    (try! (ft-transfer? usdcx-token amount sender recipient))
    (match memo to-print (print to-print) 0x)
    (ok true)
  )
)

(define-public (mint (amount uint) (recipient principal))
  (begin
    (asserts! (can-mint) ERR_UNAUTHORIZED)
    (ft-mint? usdcx-token amount recipient)
  )
)

(define-read-only (get-name) (ok (var-get token-name)))
(define-read-only (get-symbol) (ok (var-get token-symbol)))
(define-read-only (get-decimals) (ok token-decimals))
(define-read-only (get-balance (who principal)) (ok (ft-get-balance usdcx-token who)))
(define-read-only (get-total-supply) (ok (ft-get-supply usdcx-token)))
(define-read-only (get-token-uri) (ok (var-get token-uri)))
