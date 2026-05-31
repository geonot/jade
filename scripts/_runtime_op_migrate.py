#!/usr/bin/env python3
import re, sys, os
os.chdir('src/mir/lower')
files = ['store_stmt.rs', 'store_expr.rs']
for f in files:
    with open(f) as fp:
        s = fp.read()
    orig = s
    # __txn_ literal-string Call -> RuntimeOp
    s = re.sub(r'InstKind::Call\("(__txn_[a-z_]+)"\.into\(\)', r'InstKind::RuntimeOp("\1".into()', s)
    # InstKind::Call(Symbol::intern(&format!("__store_... etc.
    s = re.sub(r'InstKind::Call\(\s*Symbol::intern\(&format!\("(__(?:store|kv|vec|bloom|fts|graph|ts)_)',
               r'InstKind::RuntimeOp(Symbol::intern(&format!("\1', s)
    # also handle the `Symbol::intern(&encoded)` variant where encoded was prefixed earlier.
    # For these, we need to know the prefix from earlier in the file; safer to look for
    # `Symbol::intern(&encoded)` immediately following a `let mut encoded = format!("__store_...`.
    # Simpler: convert any Call(Symbol::intern(&encoded), ...) to RuntimeOp — these files
    # only emit store ops so it's safe.
    s = re.sub(r'InstKind::Call\(Symbol::intern\(&encoded\)', r'InstKind::RuntimeOp(Symbol::intern(&encoded)', s)
    if s != orig:
        with open(f, 'w') as fp:
            fp.write(s)
        print(f, 'changed')
    else:
        print(f, 'unchanged')
