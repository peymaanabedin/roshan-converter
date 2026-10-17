#define UNICODE
#define _UNICODE
#include <windows.h>
#include <stdio.h>

int wmain(int argc, wchar_t **argv) {
    for (int index=1; index<argc; index++) {
        SetLastError(0);
        DWORD attr=GetFileAttributesW(argv[index]);
        DWORD attr_error=GetLastError();
        WIN32_FIND_DATAW data={0};
        SetLastError(0);
        HANDLE find=FindFirstFileW(argv[index],&data);
        DWORD find_error=GetLastError();
        ULARGE_INTEGER size={0},write={0};
        size.HighPart=data.nFileSizeHigh;size.LowPart=data.nFileSizeLow;
        write.HighPart=data.ftLastWriteTime.dwHighDateTime;write.LowPart=data.ftLastWriteTime.dwLowDateTime;
        printf("{\"index\":%d,\"get_attributes\":%lu,\"attributes_error\":%lu,\"find_success\":%s,\"find_error\":%lu,\"bytes\":%llu,\"write_filetime\":%llu}\n",
               index,attr,attr_error,find!=INVALID_HANDLE_VALUE?"true":"false",find_error,
               (unsigned long long)size.QuadPart,(unsigned long long)write.QuadPart);
        if(find!=INVALID_HANDLE_VALUE)FindClose(find);
    }
    return 0;
}
