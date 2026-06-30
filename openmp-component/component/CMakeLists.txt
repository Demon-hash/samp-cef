cmake_minimum_required(VERSION 3.19)
project(samp-cef-openmp-component LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

set(SAMP_CEF_ROOT "${CMAKE_CURRENT_LIST_DIR}/../.." CACHE PATH "Path to the samp-cef repository")
set(OPEN_MP_ROOT "$ENV{OPEN_MP_ROOT}" CACHE PATH "Path to the open.mp repository")

set(OPENMP_EXT_ROOT "$ENV{OPENMP_EXT_ROOT}" CACHE PATH "Path to the openmp-ext gRPC component repository")
set(OPENMP_EXT_BUILD_DIR "$ENV{OPENMP_EXT_BUILD_DIR}" CACHE PATH "Path to the openmp-ext build directory")
if(NOT OPENMP_EXT_BUILD_DIR AND OPENMP_EXT_ROOT)
	set(OPENMP_EXT_BUILD_DIR "${OPENMP_EXT_ROOT}/build")
endif()
set(SAMP_CEF_ENABLE_GRPC_EXT "OFF" CACHE STRING "Enable openmp-ext gRPC integration: AUTO, ON, or OFF")
set_property(CACHE SAMP_CEF_ENABLE_GRPC_EXT PROPERTY STRINGS AUTO ON OFF)
set(OPEN_MP_SERVER_ROOT "$ENV{OPEN_MP_SERVER_ROOT}" CACHE PATH "Optional open.mp server root used by the install target")
set(SAMP_CEF_INSTALL_COMPONENT_DIR "" CACHE PATH "Directory where open.mp component binaries are installed")
set(SAMP_CEF_INSTALL_COMPONENT_NAME "samp-cef-openmp" CACHE STRING "CMake install component used for CEF open.mp artifacts")
set(SAMP_CEF_INSTALL_GRPC_EXT ON CACHE BOOL "Install omp-grpc next to cef when gRPC integration is enabled")
set(SAMP_CEF_SUPPRESS_DEPENDENCY_CMAKE_DEPRECATION_WARNINGS ON CACHE BOOL "Suppress CMake deprecation warnings from dependency projects")

function(samp_cef_append_loader_rpath target_name)
	if(APPLE)
		set_property(TARGET "${target_name}" APPEND PROPERTY BUILD_RPATH "@loader_path")
		set_property(TARGET "${target_name}" APPEND PROPERTY INSTALL_RPATH "@loader_path")
	elseif(UNIX)
		set_property(TARGET "${target_name}" APPEND PROPERTY BUILD_RPATH "$ORIGIN")
		set_property(TARGET "${target_name}" APPEND PROPERTY INSTALL_RPATH "$ORIGIN")
	endif()
endfunction()

function(samp_cef_find_grpc_dependencies output_variable)
	find_package(Protobuf CONFIG QUIET)
	if(NOT Protobuf_FOUND)
		find_package(Protobuf MODULE QUIET)
	endif()

	find_package(gRPC CONFIG QUIET)
	if(NOT gRPC_FOUND)
		find_package(PkgConfig QUIET)
		if(PkgConfig_FOUND)
			pkg_check_modules(GRPCPP QUIET IMPORTED_TARGET grpc++)
			if(GRPCPP_FOUND AND TARGET PkgConfig::GRPCPP AND NOT TARGET gRPC::grpc++)
				add_library(gRPC::grpc++ INTERFACE IMPORTED)
				target_link_libraries(gRPC::grpc++ INTERFACE PkgConfig::GRPCPP)
			endif()
		endif()
	endif()

	if(TARGET protobuf::libprotobuf AND TARGET gRPC::grpc++)
		set("${output_variable}" TRUE PARENT_SCOPE)
	else()
		set("${output_variable}" FALSE PARENT_SCOPE)
	endif()
endfunction()

function(samp_cef_import_omp_grpc output_variable)
	if(NOT TARGET samp_cef_omp_grpc)
		add_library(samp_cef_omp_grpc SHARED IMPORTED GLOBAL)
	endif()

	set_target_properties(samp_cef_omp_grpc PROPERTIES
		IMPORTED_LOCATION "${OPENMP_EXT_GRPC_LIBRARY}"
	)

	if(WIN32)
		set_target_properties(samp_cef_omp_grpc PROPERTIES
			IMPORTED_IMPLIB "${OPENMP_EXT_GRPC_IMPLIB}"
		)
	endif()

	set("${output_variable}" samp_cef_omp_grpc PARENT_SCOPE)
endfunction()

if(NOT EXISTS "${OPEN_MP_ROOT}/SDK/CMakeLists.txt")
	message(FATAL_ERROR "OPEN_MP_ROOT must point to an open.mp checkout")
endif()

find_program(CARGO_EXECUTABLE cargo REQUIRED)

set(CMAKE_POLICY_VERSION_MINIMUM 3.5)
if(SAMP_CEF_SUPPRESS_DEPENDENCY_CMAKE_DEPRECATION_WARNINGS)
	if(DEFINED CMAKE_WARN_DEPRECATED)
		set(SAMP_CEF_PREVIOUS_CMAKE_WARN_DEPRECATED_DEFINED TRUE)
		set(SAMP_CEF_PREVIOUS_CMAKE_WARN_DEPRECATED "${CMAKE_WARN_DEPRECATED}")
	else()
		set(SAMP_CEF_PREVIOUS_CMAKE_WARN_DEPRECATED_DEFINED FALSE)
	endif()
	set(CMAKE_WARN_DEPRECATED OFF)
endif()
add_subdirectory("${OPEN_MP_ROOT}/SDK" "${CMAKE_CURRENT_BINARY_DIR}/openmp-sdk")
if(SAMP_CEF_SUPPRESS_DEPENDENCY_CMAKE_DEPRECATION_WARNINGS)
	if(SAMP_CEF_PREVIOUS_CMAKE_WARN_DEPRECATED_DEFINED)
		set(CMAKE_WARN_DEPRECATED "${SAMP_CEF_PREVIOUS_CMAKE_WARN_DEPRECATED}")
	else()
		unset(CMAKE_WARN_DEPRECATED)
	endif()
endif()

set(SAMP_CEF_RUST_TARGET "" CACHE STRING "Optional Cargo target triple for the Rust static library")
set(SAMP_CEF_CARGO_TARGET_DIR "${CMAKE_CURRENT_BINARY_DIR}/cargo-target" CACHE PATH "Cargo target directory")
set(RUST_MANIFEST "${SAMP_CEF_ROOT}/openmp-component/Cargo.toml")
set(GENERATED_INCLUDE_DIR "${CMAKE_CURRENT_BINARY_DIR}/generated/include")
set(GENERATED_CXXBRIDGE_HEADER "${GENERATED_INCLUDE_DIR}/samp-cef-openmp/src/lib.rs.h")

set(CARGO_BUILD_ENV
	"CARGO_TARGET_DIR=${SAMP_CEF_CARGO_TARGET_DIR}"
	"MAKEFLAGS="
)
if(APPLE AND CMAKE_OSX_DEPLOYMENT_TARGET)
	list(APPEND CARGO_BUILD_ENV "MACOSX_DEPLOYMENT_TARGET=${CMAKE_OSX_DEPLOYMENT_TARGET}")
endif()

set(CARGO_BUILD_ARGS build --release --manifest-path "${RUST_MANIFEST}")
if(SAMP_CEF_RUST_TARGET)
	list(APPEND CARGO_BUILD_ARGS --target "${SAMP_CEF_RUST_TARGET}")
	set(RUST_LIB_DIR "${SAMP_CEF_CARGO_TARGET_DIR}/${SAMP_CEF_RUST_TARGET}/release")
else()
	set(RUST_LIB_DIR "${SAMP_CEF_CARGO_TARGET_DIR}/release")
endif()

if(WIN32)
	set(RUST_STATIC_LIB "${RUST_LIB_DIR}/samp_cef_openmp.lib")
else()
	set(RUST_STATIC_LIB "${RUST_LIB_DIR}/libsamp_cef_openmp.a")
endif()

add_custom_command(
	OUTPUT
		"${RUST_STATIC_LIB}"
		"${GENERATED_CXXBRIDGE_HEADER}"
	COMMAND
		"${CMAKE_COMMAND}" -E env
			${CARGO_BUILD_ENV}
			"${CARGO_EXECUTABLE}" ${CARGO_BUILD_ARGS}
	COMMAND
		"${CMAKE_COMMAND}"
			"-DRUST_LIB_DIR=${RUST_LIB_DIR}"
			"-DGENERATED_INCLUDE_DIR=${GENERATED_INCLUDE_DIR}"
			-P "${CMAKE_CURRENT_LIST_DIR}/copy_cxxbridge_header.cmake"
	WORKING_DIRECTORY "${SAMP_CEF_ROOT}"
	DEPENDS
		"${SAMP_CEF_ROOT}/Cargo.toml"
		"${SAMP_CEF_ROOT}/Cargo.lock"
		"${RUST_MANIFEST}"
		"${SAMP_CEF_ROOT}/openmp-component/build.rs"
		"${SAMP_CEF_ROOT}/openmp-component/src/lib.rs"
		"${SAMP_CEF_ROOT}/server-core/src/lib.rs"
		"${SAMP_CEF_ROOT}/server-core/src/server.rs"
		"${SAMP_CEF_ROOT}/server-core/src/client.rs"
	VERBATIM
)

add_custom_target(samp_cef_openmp_rust DEPENDS "${RUST_STATIC_LIB}" "${GENERATED_CXXBRIDGE_HEADER}")

add_library(cef SHARED
	src/component.cpp
)

set(SAMP_CEF_GRPC_EXT_ENABLED FALSE)

add_dependencies(cef samp_cef_openmp_rust)

target_include_directories(cef PRIVATE
	"${GENERATED_INCLUDE_DIR}"
)

target_include_directories(cef SYSTEM PRIVATE
	"${OPEN_MP_ROOT}/lib/pawn/source"
	"${OPEN_MP_ROOT}/lib/pawn/source/linux"
)

target_link_libraries(cef PRIVATE
	OMP-SDK
	"${RUST_STATIC_LIB}"
)

target_compile_definitions(cef PRIVATE
	OMP_VERSION_MAJOR=1
	OMP_VERSION_MINOR=5
	OMP_VERSION_PATCH=8
	FLOATPOINT
	PAWN_CELL_SIZE=32
)

string(TOUPPER "${SAMP_CEF_ENABLE_GRPC_EXT}" SAMP_CEF_ENABLE_GRPC_EXT_UPPER)
set(OPENMP_EXT_PROTO_DIR "${OPENMP_EXT_BUILD_DIR}/generated/proto")
set(OPENMP_EXT_GRPC_DEFAULT_LIBRARY "${OPENMP_EXT_BUILD_DIR}/omp-grpc${CMAKE_SHARED_LIBRARY_SUFFIX}")
set(OPENMP_EXT_GRPC_LIBRARY "${OPENMP_EXT_GRPC_DEFAULT_LIBRARY}" CACHE FILEPATH "Path to the built omp-grpc shared library")
if(WIN32)
	set(OPENMP_EXT_GRPC_DEFAULT_IMPLIB "${OPENMP_EXT_BUILD_DIR}/${CMAKE_IMPORT_LIBRARY_PREFIX}omp-grpc${CMAKE_IMPORT_LIBRARY_SUFFIX}")
	set(OPENMP_EXT_GRPC_IMPLIB "${OPENMP_EXT_GRPC_DEFAULT_IMPLIB}" CACHE FILEPATH "Path to the omp-grpc import library")
endif()

set(OPENMP_EXT_AVAILABLE FALSE)
set(OPENMP_EXT_MISSING "")
if(NOT OPENMP_EXT_ROOT)
	list(APPEND OPENMP_EXT_MISSING "OPENMP_EXT_ROOT")
elseif(NOT EXISTS "${OPENMP_EXT_ROOT}/include/omp_ext/component.hpp")
	list(APPEND OPENMP_EXT_MISSING "${OPENMP_EXT_ROOT}/include/omp_ext/component.hpp")
endif()
if(NOT OPENMP_EXT_BUILD_DIR)
	list(APPEND OPENMP_EXT_MISSING "OPENMP_EXT_BUILD_DIR")
else()
	foreach(required_file IN ITEMS
		"${OPENMP_EXT_PROTO_DIR}/omp_ext.pb.h"
		"${OPENMP_EXT_PROTO_DIR}/omp_ext.grpc.pb.h"
		"${OPENMP_EXT_GRPC_LIBRARY}"
	)
		if(NOT EXISTS "${required_file}")
			list(APPEND OPENMP_EXT_MISSING "${required_file}")
		endif()
	endforeach()
	if(WIN32 AND NOT EXISTS "${OPENMP_EXT_GRPC_IMPLIB}")
		list(APPEND OPENMP_EXT_MISSING "${OPENMP_EXT_GRPC_IMPLIB}")
	endif()
endif()
if(NOT OPENMP_EXT_MISSING)
	set(OPENMP_EXT_AVAILABLE TRUE)
endif()
if(OPENMP_EXT_MISSING)
	string(REPLACE ";" "\n  - " OPENMP_EXT_MISSING_REPORT "${OPENMP_EXT_MISSING}")
	set(OPENMP_EXT_MISSING_REPORT "\n  - ${OPENMP_EXT_MISSING_REPORT}")
endif()

if(SAMP_CEF_ENABLE_GRPC_EXT_UPPER STREQUAL "ON" OR (SAMP_CEF_ENABLE_GRPC_EXT_UPPER STREQUAL "AUTO" AND OPENMP_EXT_AVAILABLE))
	if(NOT OPENMP_EXT_AVAILABLE)
		message(FATAL_ERROR "SAMP_CEF_ENABLE_GRPC_EXT is ON, but openmp-ext headers/proto/library were not found:${OPENMP_EXT_MISSING_REPORT}\nSet OPENMP_EXT_ROOT/OPENMP_EXT_BUILD_DIR or use -DSAMP_CEF_ENABLE_GRPC_EXT=OFF.")
	endif()

	samp_cef_find_grpc_dependencies(SAMP_CEF_GRPC_DEPS_AVAILABLE)
	if(NOT SAMP_CEF_GRPC_DEPS_AVAILABLE)
		if(SAMP_CEF_ENABLE_GRPC_EXT_UPPER STREQUAL "ON")
			message(FATAL_ERROR "SAMP_CEF_ENABLE_GRPC_EXT is ON, but Protobuf/gRPC CMake packages were not found. Set CMAKE_PREFIX_PATH or use -DSAMP_CEF_ENABLE_GRPC_EXT=OFF.")
		endif()
		message(STATUS "openmp-ext gRPC integration disabled: Protobuf/gRPC CMake packages were not found")
	else()
		samp_cef_import_omp_grpc(OPENMP_EXT_GRPC_TARGET)

		set(SAMP_CEF_GRPC_EXT_ENABLED TRUE)
		message(STATUS "openmp-ext gRPC integration enabled: ${OPENMP_EXT_GRPC_LIBRARY}")
		target_compile_definitions(cef PRIVATE SAMP_CEF_HAS_GRPC_EXT=1)
		target_include_directories(cef SYSTEM PRIVATE
			"${OPENMP_EXT_ROOT}/include"
			"${OPENMP_EXT_PROTO_DIR}"
			"${OPEN_MP_ROOT}/CAPI/include"
		)
		target_link_libraries(cef PRIVATE
			"${OPENMP_EXT_GRPC_TARGET}"
			protobuf::libprotobuf
			gRPC::grpc++
		)

		samp_cef_append_loader_rpath(cef)
	endif()
elseif(SAMP_CEF_ENABLE_GRPC_EXT_UPPER STREQUAL "AUTO")
	message(STATUS "openmp-ext gRPC integration disabled: headers/proto/library were not found${OPENMP_EXT_MISSING_REPORT}")
elseif(NOT SAMP_CEF_ENABLE_GRPC_EXT_UPPER STREQUAL "OFF")
	message(FATAL_ERROR "SAMP_CEF_ENABLE_GRPC_EXT must be AUTO, ON, or OFF")
endif()

if(UNIX)
	target_compile_definitions(cef PRIVATE LINUX)
	if(NOT APPLE)
		target_compile_options(cef PRIVATE -fno-eliminate-unused-debug-types)
	endif()
endif()

if(APPLE)
	find_library(SECURITY_FRAMEWORK Security REQUIRED)
	find_library(COREFOUNDATION_FRAMEWORK CoreFoundation REQUIRED)
	target_link_libraries(cef PRIVATE
		"${SECURITY_FRAMEWORK}"
		"${COREFOUNDATION_FRAMEWORK}"
	)
endif()

set_property(TARGET cef PROPERTY OUTPUT_NAME cef)
set_property(TARGET cef PROPERTY PREFIX "")

if(NOT SAMP_CEF_INSTALL_COMPONENT_DIR)
	if(OPEN_MP_SERVER_ROOT)
		set(SAMP_CEF_INSTALL_COMPONENT_DIR "${OPEN_MP_SERVER_ROOT}/components")
	else()
		set(SAMP_CEF_INSTALL_COMPONENT_DIR "components")
	endif()
endif()

install(TARGETS cef
	RUNTIME DESTINATION "${SAMP_CEF_INSTALL_COMPONENT_DIR}" COMPONENT "${SAMP_CEF_INSTALL_COMPONENT_NAME}"
	LIBRARY DESTINATION "${SAMP_CEF_INSTALL_COMPONENT_DIR}" COMPONENT "${SAMP_CEF_INSTALL_COMPONENT_NAME}"
	ARCHIVE DESTINATION "${SAMP_CEF_INSTALL_COMPONENT_DIR}" COMPONENT "${SAMP_CEF_INSTALL_COMPONENT_NAME}"
)

if(SAMP_CEF_GRPC_EXT_ENABLED AND SAMP_CEF_INSTALL_GRPC_EXT)
	install(FILES "${OPENMP_EXT_GRPC_LIBRARY}" DESTINATION "${SAMP_CEF_INSTALL_COMPONENT_DIR}" COMPONENT "${SAMP_CEF_INSTALL_COMPONENT_NAME}")
endif()
