(component
  (type $interface (instance (export "forbidden" (func))))
  (import "wasi:sockets/tcp@0.2.0" (instance (type $interface)))
)
