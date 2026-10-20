int a01(void (*cb)(int)) { f01(); }
int a02(void (cb)(int)) { f02(); }
int a03(void (*cb)) { f03(); }
int a04(int (*cb)(int (x))) { f04(); }
int a05(void ( * cb )( int )) { f05(); }
int a06(void (*cb)
(int)) { f06(); }
int a07(void (*cb)(int, char *s)) { f07(); }
int a08(void (**cb)(int)) { f08(); }
int a09(void (*cb[4])(int)) { f09(); }
int a10((x)) { f10(); }
int a11(void (*)(int)) { f11(); }
int a12(int x, void (*cb)(int), int (*cb2)(char)) { f12(); }
int a13(void (*cb)(int)/*c*/) { f13(); }
int a14(void(*cb)(int)) { f14(); }
int a15(void (* /* c */ cb)(int)) { f15(); }
int a16(void (*cb)(void (*)(int))) { f16(); }
int a17(void (*cb)(), int y) { f17(); }
int a18(void (*cb)(int)
) { f18(); }
int a19(void (*cb)(int x,
int y)) { f19(); }
int a20(void (*cb)(int) , void (*cc)(int)) { f20(); }
int a21(int x) __attribute__((unused));
int a22(void (callback)(dict*)) { f22(); }
int a23(void (*callback)(dict*)) { f23(); }
int a24(void (*callback)(dict *)) { f24(); }
int a25(void (*callback)(dict)) { f25(); }
int a26(void (x)) { f26(); }
int a27(void (*x)[3]) { f27(); }
int a28((*x)(y)) { f28(); }
