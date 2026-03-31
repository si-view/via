;;; via.il — SKILL bridge for the `via` IPC tool
;;;
;;; Connects Virtuoso to external processes via the `via` binary.
;;; In IC design a "via" punches through layers to carry a signal — here it
;;; carries SKILL expressions and JSON results between Virtuoso and the outside.
;;;
;;; Usage:
;;;   load("/path/to/via.il")
;;;   si_view_start("/usr/local/bin/via")
;;;   si_view_start("/usr/local/bin/via" ?secret "your-secret")
;;;   ; → CIW: [si-view] started  pid=…  bridge=…  sock=/tmp/via-<user>.sock
;;;
;;;   ; Third-party calls:
;;;   ;   via send --sock /tmp/via-<user>.sock --expr 'hiGetPoint()'
;;;   ;   via send --sock ... --secret your-secret --expr '...'
;;;
;;;   si_view_emit("progress" 75)
;;;   si_view_stop()
;;;
;;; IPC protocol
;;;   via serve stdout → Virtuoso:      SKILL_EXPRESSION\n
;;;   Virtuoso → via forward stdin:     S:JSON_VALUE\n  |  F:error\n  |  E:{…}\n
;;;   via forward → callback socket:    <TOKEN> S:JSON_VALUE\n  (etc.)
;;;
;;; Security
;;;   --cb-token  is generated fresh per session (local let-binding, never global).
;;;   --secret    is optional; omit for development / trusted local use.
;;;   Both are simple shared-secret tokens passed as CLI arguments.
;;;
;;; Note: --secret must not contain spaces or shell metacharacters.
;;;       A 32-character hex string is ideal.

;;; ── globals ───────────────────────────────────────────────────────────────────

_via_server_pid  = nil                     ; `via serve`   process PID
_via_bridge_pid  = nil                     ; `via forward` process PID
_via_ipc_buf     = ""                      ; line-reassembly buffer
_via_remote_tbl  = makeTable("_via_remote" nil) ; id → opaque object

;;; ── public API ────────────────────────────────────────────────────────────────

;;; Start `via serve` and `via forward` as Virtuoso child processes.
;;;
;;; binary_path  — absolute path to the `via` binary
;;; ?sock        — Unix socket path clients connect to
;;;                [default: /tmp/via-<USER>.sock]
;;; ?secret      — shared secret for `via send` authentication
;;;                (omit or pass "" to disable authentication)
procedure(si_view_start(binary_path
    @key
    (sock    sprintf(nil "/tmp/via-%s.sock" getShellEnvVar("USER")))
    (secret  ""))

  let((vpid cb_token cb_sock serve_cmd serve_pid fwd_cmd bridge_pid)

    vpid     = ipcGetPid()
    cb_token = _via_gen_token()      ; local — discarded when let-block ends
    cb_sock  = sprintf(nil "/tmp/via-cb-%d.sock" vpid)

    ;;; ── serve process ─────────────────────────────────────────────────────────
    ;;; Only pass --secret when a non-empty secret is configured, so that the
    ;;; binary receives a well-formed argument list in both cases.
    if(secret != "" then
      serve_cmd = sprintf(nil
        "%s serve --sock %s --cb-sock %s --cb-token %s --secret %s"
        binary_path sock cb_sock cb_token secret)
    else
      serve_cmd = sprintf(nil
        "%s serve --sock %s --cb-sock %s --cb-token %s"
        binary_path sock cb_sock cb_token)
    )
    serve_pid = ipcBeginProcess(serve_cmd "" 'si_view_on_data "" t)
    if(!serve_pid then
      error("[si-view] failed to start serve: %s" binary_path)
    )
    _via_server_pid = serve_pid

    ;;; ── forward process ───────────────────────────────────────────────────────
    ;;; ipcWriteProcess(serve_pid …) is unreliable for reading back; the forwarder
    ;;; bridges Virtuoso → callback socket instead.
    fwd_cmd = sprintf(nil
      "%s forward --cb-sock %s --cb-token %s"
      binary_path cb_sock cb_token)
    bridge_pid = ipcBeginProcess(fwd_cmd "" nil "" t)
    if(!bridge_pid then
      ipcKillProcess(_via_server_pid)
      _via_server_pid = nil
      error("[si-view] failed to start forward: %s" binary_path)
    )
    _via_bridge_pid = bridge_pid

    printf("[si-view] started  pid=%d  bridge=%d  sock=%s\n"
      serve_pid bridge_pid sock)
    t
  )
)

;;; Stop both child processes.
procedure(si_view_stop()
  when(_via_server_pid
    ipcKillProcess(_via_server_pid)
    _via_server_pid = nil
  )
  when(_via_bridge_pid
    ipcKillProcess(_via_bridge_pid)
    _via_bridge_pid = nil
  )
  printf("[si-view] stopped\n")
)

;;; Push a named event with a SKILL value.
;;; The event is currently logged on the server side.
procedure(si_view_emit(name val)
  when(_via_bridge_pid
    ipcWriteProcess(_via_bridge_pid
      sprintf(nil "E:{\"name\":\"%s\",\"value\":%s}\n"
        _via_esc(name)
        _via_to_json(val)))
  )
)

;;; ── IPC callback ──────────────────────────────────────────────────────────────

;;; Called by Virtuoso when `via serve` writes a SKILL expression to stdout.
;;; Evaluates the expression and sends the result to the forwarder.
;;; Both evaluation and serialisation are wrapped in errset so that any SKILL
;;; error produces a well-formed F: response rather than a silent failure.
procedure(si_view_on_data(pid data)
  let((pos line result json_result errMsg reply)
    _via_ipc_buf = strcat(_via_ipc_buf data)
    pos = _via_find_nl(_via_ipc_buf)
    while(pos
      line         = substring(_via_ipc_buf 1 pos - 1)
      _via_ipc_buf = if(strlen(_via_ipc_buf) > pos then
                       substring(_via_ipc_buf pos + 1 strlen(_via_ipc_buf))
                     else
                       "")

      ;;; Step 1 — evaluate
      result = errset(evalstring(line) t)
      if(result then
        ;;; Step 2 — serialise (may fail for opaque objects; errset catches that)
        json_result = errset(_via_to_typed_json(car(result)) t)
        if(json_result then
          reply = strcat("S:" car(json_result) "\n")
        else
          reply = strcat("F:serialization error\n")
        )
      else
        errMsg = if(boundp('errText) && listp(errText)
                      && errText && stringp(car(errText)) then
                   car(errText)
                 else
                   "SKILL evaluation error")
        reply = strcat("F:" _via_esc(errMsg) "\n")
      )
      ipcWriteProcess(_via_bridge_pid reply)
      pos = _via_find_nl(_via_ipc_buf)
    )
  )
)

;;; ── internal helpers ──────────────────────────────────────────────────────────

;;; Generate a 32-character random hex token (local use only).
procedure(_via_gen_token()
  buildString(
    list(
      sprintf(nil "%04x" random(65536))
      sprintf(nil "%04x" random(65536))
      sprintf(nil "%04x" random(65536))
      sprintf(nil "%04x" random(65536))
      sprintf(nil "%04x" random(65536))
      sprintf(nil "%04x" random(65536))
      sprintf(nil "%04x" random(65536))
      sprintf(nil "%04x" random(65536)))
    "")
)

;;; 1-based index of first '\n' in str, or nil.
procedure(_via_find_nl(str)
  let((len i found)
    len   = strlen(str)
    i     = 1
    found = nil
    while(i <= len && !found
      if(substring(str i 1) == "\n" then
        found = i
      else
        i++
      )
    )
    found
  )
)

;;; ── SKILL → JSON ──────────────────────────────────────────────────────────────

;;; Return an EvalResult JSON string matching the Rust EvalResult struct:
;;;   {"data":<value>,"is_ref":false,"code":0}
;;;   {"data":{"id":"…","kind":"…"},"is_ref":true,"code":0}
;;;
;;; The SKILL type is carried implicitly by the JSON shape; `is_ref` flags
;;; remote/opaque objects so the caller knows `data` is a handle stub.
procedure(_via_to_typed_json(val)
  let((is_ref value_json)
    is_ref = nil
    cond(
      (null(val)
        value_json = "null")
      ((val == t)
        value_json = "true")
      (integerp(val)
        value_json = sprintf(nil "%d" val))
      (floatp(val)
        value_json = _via_fmt_float(val))
      (stringp(val)
        value_json = sprintf(nil "\"%s\"" _via_esc(val)))
      (symbolp(val)
        value_json = sprintf(nil "\"%s\"" _via_esc(symbolToString(val))))
      (listp(val)
        if(_via_is_plist(val) then
          value_json = _via_plist_to_json(val)
        else
          value_json = _via_list_to_json(val)
        ))
      (tablep(val)
        value_json = _via_table_to_json(val))
      (t
        is_ref     = t
        let((id kind)
          id   = sprintf(nil "%A" val)
          kind = symbolToString(type(val))
          _via_remote_tbl[id] = val
          value_json = sprintf(nil "{\"id\":\"%s\",\"kind\":\"%s\"}"
                         _via_esc(id) kind)
        ))
    )
    sprintf(nil "{\"data\":%s,\"is_ref\":%s,\"code\":0}"
      value_json
      if(is_ref then "true" else "false"))
  )
)

procedure(_via_to_json(val)
  cond(
    (null(val)     "null")
    ((val == t)    "true")
    (integerp(val) sprintf(nil "%d" val))
    (floatp(val)   _via_fmt_float(val))
    (stringp(val)  sprintf(nil "\"%s\"" _via_esc(val)))
    (symbolp(val)  sprintf(nil "{\"__sym\":\"%s\"}" val))
    (listp(val)    if(_via_is_plist(val) then
                     _via_plist_to_json(val)
                   else
                     _via_list_to_json(val)))
    (tablep(val)   _via_table_to_json(val))
    (t             _via_remote_to_json(val))
  )
)

procedure(_via_fmt_float(v)
  let((s)
    s = sprintf(nil "%g" v)
    if(index(s ".") || index(s "e") || index(s "E") then
      s
    else
      strcat(s ".0"))
  )
)

;;; Escape a SKILL string for embedding in a JSON string literal.
procedure(_via_esc(s)
  let((i len ch out)
    out = ""
    len = strlen(s)
    for(i 1 len
      ch = substring(s i 1)
      cond(
        ((ch == "\\") out = strcat(out "\\\\"))
        ((ch == "\"") out = strcat(out "\\\""))
        ((ch == "\n") out = strcat(out "\\n"))
        ((ch == "\t") out = strcat(out "\\t"))
        (t            out = strcat(out ch))
      )
    )
    out
  )
)

procedure(_via_list_to_json(lst)
  sprintf(nil "[%s]" buildString(mapcar('_via_to_json lst) ","))
)

;;; A plist is a list of even length whose odd-indexed elements are symbols.
;;; It serialises to a JSON object: (x 1 y 2) → {"x":1,"y":2}
procedure(_via_is_plist(lst)
  let((len ok i)
    len = length(lst)
    ok  = len > 0 && mod(len 2) == 0
    i   = 1
    while(ok && i <= len
      if(symbolp(nth(i - 1 lst)) then
        i = i + 2
      else
        ok = nil
      )
    )
    ok
  )
)

procedure(_via_plist_to_json(lst)
  let((pairs i len)
    pairs = list()
    len   = length(lst)
    i     = 1
    while(i < len
      pairs = append1(pairs
        sprintf(nil "\"%s\":%s" nth(i - 1 lst) _via_to_json(nth(i lst))))
      i = i + 2
    )
    sprintf(nil "{%s}" buildString(pairs ","))
  )
)

procedure(_via_table_to_json(tbl)
  let((pairs)
    pairs = list()
    foreach(key tbl
      pairs = append1(pairs
        sprintf(nil "\"%s\":%s"
          _via_esc(sprintf(nil "%s" key))
          _via_to_json(tbl[key])))
    )
    sprintf(nil "{%s}" buildString(pairs ","))
  )
)

;;; Store an opaque object (dbObject, cellView, etc.) in _via_remote_tbl and
;;; return a JSON stub for use inside _via_to_json (plain value context).
;;;
;;; The id is the %A representation of the object — human-readable and stable
;;; within a session.  To retrieve the object later, evaluate:
;;;   _via_remote_tbl["<id>"]
procedure(_via_remote_to_json(val)
  let((id kind)
    id   = sprintf(nil "%A" val)
    kind = symbolToString(type(val))
    _via_remote_tbl[id] = val
    sprintf(nil "{\"__remote\":\"%s\",\"__kind\":\"%s\"}"
      _via_esc(id) kind)
  )
)
