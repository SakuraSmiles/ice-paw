call "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvarsall.bat" amd64
set SODIUM_LIB_DIR=D:\workspace\ice-paw\sodium-prebuilt\libsodium\x64\Release\v143\static
set SODIUM_STATIC=true
set PATH=D:\Program Files\nodejs;D:\Program Files\Git\bin;C:\Users\dabai\AppData\Roaming\npm;%PATH%
cd /d D:\workspace\ice-paw
echo Starting pnpm tauri:dev...
call pnpm tauri:dev
echo Exit code: %ERRORLEVEL%