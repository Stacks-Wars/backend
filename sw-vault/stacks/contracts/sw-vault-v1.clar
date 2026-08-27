;; title: sw-vault-v1
;; summary: Central USDCx vault for Stacks Wars lobbies (normal + sponsored).
;; description:
;;   First join sets entry fee and sponsored flag for a lobby path.
;;   Normal joins transfer entry; sponsored follow-ups join free.
;;   leave/kick require oracle signatures and refund paid amounts.

(define-constant PLATFORM-WALLET 'SP299MBHT7FPPP2SKEY73V4DHW67467SED87A4HH4)
(define-constant PLATFORM-FEE u2)
(define-constant TRUSTED-PUBLIC-KEY 0x03cd2cfdbd2ad9332828a7a13ef62cb999e063421c708e863a7ffed71fb61c88c9)

(define-constant ERR-EMPTY-PATH (err u200))
(define-constant ERR-ZERO-ENTRY (err u201))
(define-constant ERR-ALREADY-JOINED (err u202))
(define-constant ERR-NOT-JOINED (err u203))
(define-constant ERR-ENTRY-MISMATCH (err u204))
(define-constant ERR-SPONSORED-MISMATCH (err u205))
(define-constant ERR-INVALID-SIGNATURE (err u206))
(define-constant ERR-MESSAGE-HASH-FAILED (err u207))
(define-constant ERR-CREATOR-NOT-LAST (err u208))
(define-constant ERR-CLAIMS-STARTED (err u209))
(define-constant ERR-ZERO-AMOUNT (err u210))
(define-constant ERR-INSUFFICIENT-POT (err u211))
(define-constant ERR-NONCE-USED (err u212))
(define-constant ERR-PLAYER-MISMATCH (err u213))
(define-constant ERR-UNKNOWN-LOBBY (err u214))
(define-constant ERR-INVALID-SPLIT (err u215))

(define-map lobby-entry (string-ascii 64) uint)
(define-map lobby-sponsored (string-ascii 64) bool)
(define-map lobby-creator (string-ascii 64) principal)
(define-map lobby-pot (string-ascii 64) uint)
(define-map lobby-player-count (string-ascii 64) uint)
(define-map lobby-claims-started (string-ascii 64) bool)
(define-map lobby-players
  { path: (string-ascii 64), player: principal }
  uint
)
(define-map lobby-claim-nonces
  { path: (string-ascii 64), nonce: uint }
  bool
)

(define-private (path-ok (lobby-path (string-ascii 64)))
  (> (len lobby-path) u0)
)

(define-private (lobby-exists (lobby-path (string-ascii 64)))
  (is-some (map-get? lobby-entry lobby-path))
)

(define-private (claims-started (lobby-path (string-ascii 64)))
  (default-to false (map-get? lobby-claims-started lobby-path))
)

(define-private (clear-lobby (lobby-path (string-ascii 64)))
  (begin
    (map-delete lobby-entry lobby-path)
    (map-delete lobby-sponsored lobby-path)
    (map-delete lobby-creator lobby-path)
    (map-delete lobby-pot lobby-path)
    (map-delete lobby-player-count lobby-path)
    (map-delete lobby-claims-started lobby-path)
    true
  )
)

(define-private (construct-message-hash
  (action (string-ascii 16))
  (lobby-path (string-ascii 64))
  (player principal)
  (amount uint)
  (nonce uint)
)
  (let (
    (message {
      action: action,
      lobby-path: lobby-path,
      player: player,
      amount: amount,
      nonce: nonce
    })
  )
    (match (to-consensus-buff? message)
      buff (ok (sha256 buff))
      ERR-MESSAGE-HASH-FAILED
    )
  )
)

(define-private (construct-claim-message-hash
  (lobby-path (string-ascii 64))
  (player principal)
  (amount uint)
  (nonce uint)
  (dev-wallet principal)
  (dev-fee uint)
)
  (let (
    (message {
      action: "claim",
      lobby-path: lobby-path,
      player: player,
      amount: amount,
      nonce: nonce,
      dev-wallet: dev-wallet,
      dev-fee: dev-fee
    })
  )
    (match (to-consensus-buff? message)
      buff (ok (sha256 buff))
      ERR-MESSAGE-HASH-FAILED
    )
  )
)

(define-private (verify-oracle
  (action (string-ascii 16))
  (lobby-path (string-ascii 64))
  (player principal)
  (amount uint)
  (nonce uint)
  (signature (buff 65))
)
  (let ((message-hash (try! (construct-message-hash action lobby-path player amount nonce))))
    (asserts!
      (secp256k1-verify message-hash signature TRUSTED-PUBLIC-KEY)
      ERR-INVALID-SIGNATURE
    )
    (ok true)
  )
)

(define-private (verify-claim-oracle
  (lobby-path (string-ascii 64))
  (player principal)
  (amount uint)
  (nonce uint)
  (dev-wallet principal)
  (dev-fee uint)
  (signature (buff 65))
)
  (let (
    (message-hash
      (try! (construct-claim-message-hash lobby-path player amount nonce dev-wallet dev-fee))
    )
  )
    (asserts!
      (secp256k1-verify message-hash signature TRUSTED-PUBLIC-KEY)
      ERR-INVALID-SIGNATURE
    )
    (ok true)
  )
)

(define-private (transfer-in (amount uint) (sender principal))
  (contract-call? 'SP120SBRBQJ00MCWS7TM5R8WJNTTKD5K0HFRC2CNE.usdcx
    transfer amount sender current-contract none)
)

(define-private (transfer-out (amount uint) (recipient principal))
  (as-contract? ((with-ft 'SP120SBRBQJ00MCWS7TM5R8WJNTTKD5K0HFRC2CNE.usdcx "usdcx-token" amount))
    (try! (contract-call? 'SP120SBRBQJ00MCWS7TM5R8WJNTTKD5K0HFRC2CNE.usdcx
      transfer amount tx-sender recipient none))
  )
)

(define-read-only (calculate-split (amount uint) (dev-fee uint))
  (let (
    (platform (/ (* amount PLATFORM-FEE) u100))
    (dev (if (is-eq dev-fee u0) u0 (/ (* amount dev-fee) u100)))
  )
    (asserts! (<= (+ platform dev) amount) ERR-INVALID-SPLIT)
    (ok {
      platform: platform,
      dev: dev,
      winner: (- amount (+ platform dev))
    })
  )
)

;; Join a lobby path. First join sets entry + sponsored and always deposits.
(define-public (join (lobby-path (string-ascii 64)) (entry-amount uint) (sponsored bool))
  (let (
    (sender tx-sender)
    (exists (lobby-exists lobby-path))
  )
    (asserts! (path-ok lobby-path) ERR-EMPTY-PATH)
    (asserts! (> entry-amount u0) ERR-ZERO-ENTRY)
    (asserts! (is-none (map-get? lobby-players { path: lobby-path, player: sender })) ERR-ALREADY-JOINED)
    (asserts! (not (claims-started lobby-path)) ERR-CLAIMS-STARTED)

    (if exists
      (let (
        (stored-entry (unwrap-panic (map-get? lobby-entry lobby-path)))
        (stored-sponsored (unwrap-panic (map-get? lobby-sponsored lobby-path)))
        (count (default-to u0 (map-get? lobby-player-count lobby-path)))
        (pot (default-to u0 (map-get? lobby-pot lobby-path)))
      )
        (asserts! (is-eq entry-amount stored-entry) ERR-ENTRY-MISMATCH)
        (asserts! (is-eq sponsored stored-sponsored) ERR-SPONSORED-MISMATCH)
        (if stored-sponsored
          (begin
            (map-set lobby-players { path: lobby-path, player: sender } u0)
            (map-set lobby-player-count lobby-path (+ count u1))
            (ok true)
          )
          (begin
            (try! (transfer-in stored-entry sender))
            (map-set lobby-players { path: lobby-path, player: sender } stored-entry)
            (map-set lobby-player-count lobby-path (+ count u1))
            (map-set lobby-pot lobby-path (+ pot stored-entry))
            (ok true)
          )
        )
      )
      (begin
        (try! (transfer-in entry-amount sender))
        (map-set lobby-entry lobby-path entry-amount)
        (map-set lobby-sponsored lobby-path sponsored)
        (map-set lobby-creator lobby-path sender)
        (map-set lobby-pot lobby-path entry-amount)
        (map-set lobby-player-count lobby-path u1)
        (map-set lobby-players { path: lobby-path, player: sender } entry-amount)
        (ok true)
      )
    )
  )
)

(define-private (remove-player (lobby-path (string-ascii 64)) (player principal) (paid uint))
  (let (
    (count (unwrap! (map-get? lobby-player-count lobby-path) ERR-UNKNOWN-LOBBY))
    (pot (unwrap! (map-get? lobby-pot lobby-path) ERR-UNKNOWN-LOBBY))
  )
    (asserts! (>= pot paid) ERR-INSUFFICIENT-POT)
    (begin
      (if (> paid u0)
        (try! (transfer-out paid player))
        true
      )
      (map-delete lobby-players { path: lobby-path, player: player })
      (let ((new-count (- count u1)))
        (if (is-eq new-count u0)
          (begin
            (clear-lobby lobby-path)
            (ok true)
          )
          (begin
            (map-set lobby-player-count lobby-path new-count)
            (map-set lobby-pot lobby-path (- pot paid))
            (ok true)
          )
        )
      )
    )
  )
)

(define-public (leave (lobby-path (string-ascii 64)) (nonce uint) (signature (buff 65)))
  (let (
    (sender tx-sender)
    (paid (unwrap! (map-get? lobby-players { path: lobby-path, player: sender }) ERR-NOT-JOINED))
    (creator (unwrap! (map-get? lobby-creator lobby-path) ERR-UNKNOWN-LOBBY))
    (count (unwrap! (map-get? lobby-player-count lobby-path) ERR-UNKNOWN-LOBBY))
  )
    (asserts! (path-ok lobby-path) ERR-EMPTY-PATH)
    (asserts! (not (claims-started lobby-path)) ERR-CLAIMS-STARTED)
    (try! (verify-oracle "leave" lobby-path sender paid nonce signature))
    (asserts!
      (or (not (is-eq sender creator)) (is-eq count u1))
      ERR-CREATOR-NOT-LAST
    )
    (remove-player lobby-path sender paid)
  )
)

(define-public (kick (lobby-path (string-ascii 64)) (player principal) (nonce uint) (signature (buff 65)))
  (let (
    (paid (unwrap! (map-get? lobby-players { path: lobby-path, player: player }) ERR-NOT-JOINED))
  )
    (asserts! (path-ok lobby-path) ERR-EMPTY-PATH)
    (asserts! (not (claims-started lobby-path)) ERR-CLAIMS-STARTED)
    (try! (verify-oracle "kick" lobby-path player paid nonce signature))
    (remove-player lobby-path player paid)
  )
)

(define-public (claim
  (lobby-path (string-ascii 64))
  (amount uint)
  (nonce uint)
  (dev-wallet principal)
  (dev-fee uint)
  (signature (buff 65))
)
  (let (
    (sender tx-sender)
    (pot (unwrap! (map-get? lobby-pot lobby-path) ERR-UNKNOWN-LOBBY))
    (split (try! (calculate-split amount dev-fee)))
  )
    (asserts! (path-ok lobby-path) ERR-EMPTY-PATH)
    (asserts! (> amount u0) ERR-ZERO-AMOUNT)
    (asserts!
      (is-none (map-get? lobby-claim-nonces { path: lobby-path, nonce: nonce }))
      ERR-NONCE-USED
    )
    (asserts! (<= amount pot) ERR-INSUFFICIENT-POT)
    (try! (verify-claim-oracle lobby-path sender amount nonce dev-wallet dev-fee signature))
    (if (> (get winner split) u0)
      (try! (transfer-out (get winner split) sender))
      true
    )
    (if (> (get platform split) u0)
      (try! (transfer-out (get platform split) PLATFORM-WALLET))
      true
    )
    (if (> (get dev split) u0)
      (try! (transfer-out (get dev split) dev-wallet))
      true
    )
    (map-set lobby-claim-nonces { path: lobby-path, nonce: nonce } true)
    (map-set lobby-claims-started lobby-path true)
    (let ((new-pot (- pot amount)))
      (map-set lobby-pot lobby-path new-pot)
      (ok true)
    )
  )
)

(define-read-only (get-entry (lobby-path (string-ascii 64)))
  (map-get? lobby-entry lobby-path)
)

(define-read-only (get-pot (lobby-path (string-ascii 64)))
  (default-to u0 (map-get? lobby-pot lobby-path))
)

(define-read-only (is-sponsored (lobby-path (string-ascii 64)))
  (default-to false (map-get? lobby-sponsored lobby-path))
)

(define-read-only (get-creator (lobby-path (string-ascii 64)))
  (map-get? lobby-creator lobby-path)
)

(define-read-only (has-joined (lobby-path (string-ascii 64)) (player principal))
  (is-some (map-get? lobby-players { path: lobby-path, player: player }))
)

(define-read-only (get-paid (lobby-path (string-ascii 64)) (player principal))
  (map-get? lobby-players { path: lobby-path, player: player })
)

(define-read-only (get-player-count (lobby-path (string-ascii 64)))
  (default-to u0 (map-get? lobby-player-count lobby-path))
)

(define-read-only (get-claims-started (lobby-path (string-ascii 64)))
  (claims-started lobby-path)
)

(define-read-only (get-platform-wallet)
  PLATFORM-WALLET
)

(define-read-only (get-platform-fee)
  PLATFORM-FEE
)
