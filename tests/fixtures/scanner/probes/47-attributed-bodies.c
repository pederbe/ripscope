__declspec(dllexport) int simple(void) { first(); }
int __declspec(noinline) placed(void) { second(); }
#if DLL
__declspec(dllexport)
#endif
int
#if WINDOWS
VimMain
#else
main
#endif
(int argc, char **argv)
{
    start(argc, argv);
}
__declspec(dllexport) BOOL WINAPI DllMain(HINSTANCE inst, DWORD reason, LPVOID reserved) {
    enter(inst);
    return TRUE;
}
__declspec(dllexport) int prototype(int arg);
extern __declspec(dllimport) T imported;
__declspec(selectany) T global;
__declspec(noinline) __declspec(dllexport) int twice(void) { third(); }
__attribute__((noinline)) int gnu(void) { fourth(); }
int after;
