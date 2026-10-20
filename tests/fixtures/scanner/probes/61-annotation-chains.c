#ifdef __cplusplus
extern "C" {
#endif
DEPRECATED(3.12) API_DATA(int) DebugFlag;
DEPRECATED(3.12) API_DATA(const char *) FileEncoding;
DEPRECATED(2, 4) /* annotation */
API_DATA(unsigned long) SplitFlag;
TYPED(int) API_DATA(int) TypedTail;
NAMED(version) API_DATA(int) NamedTail;
EMPTY() API_DATA(int) EmptyTail;
CALLBACK(T (*cb)(int)) API_DATA(int) CallbackTail;
ARRAY(T [10]) API_DATA(int) ArrayTail;
DEPRECATED(3.12) implicit(int) ATTR;
DEPRECATED(3.12) int real(int value) {
    DEPRECATED(3.12);
    return value;
}
DEPRECATED(3.12) API_DATA(int) Initialised = 0;
int after;
#ifdef __cplusplus
}
#endif
DEPRECATED(3.12) API_DATA(int) Outside;
int outside_after;
