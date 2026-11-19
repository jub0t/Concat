// Copyright (C) 2020-2026 Parabola Research Limited
// SPDX-License-Identifier: MPL-2.0

#include "Instrumentation.h"

#include "Assert.h"

#if defined(__ANDROID__)
#	include <android/log.h>
#	include <unistd.h>
#elif defined(__APPLE__)
#	include <os/log.h>
#elif defined(_WIN32)
#	include <cstdio>
#	include <io.h>
#else
#	include <cstdio>
#	include <unistd.h>
#endif

#include <cstdarg>
#include <cstdlib>
#include <cstring>

#include <Eigen/Core>

namespace Bungee::Internal {

namespace {

constexpr char log_prefix[] = "Bungee: ";

void log_write(const char *text, size_t length)
{
#if defined(_WIN32)
	::_write(2, text, (int)length);
#else
	// write() is warn_unused_result on glibc; capture the result so the
	// diagnostic is silenced without a (void) cast, which glibc ignores.
	const ssize_t written = ::write(2, text, length);
	(void)written;
#endif
}

} // namespace

void Instrumentation::log(const char *format, ...)
{
#ifndef BUNGEE_NO_LOG
	if (enabled)
	{
		constexpr size_t prefix_length = sizeof(log_prefix) - 1;
		char message[4096];
		char *const text = message + prefix_length;
		constexpr size_t text_capacity = sizeof(message) - prefix_length - 1;

		va_list args;
		va_start(args, format);
#if defined(_WIN32)
		int length = _vsnprintf(text, text_capacity, format, args);
#else
		int length = vsnprintf(text, text_capacity, format, args);
#endif
		va_end(args);

		if (length < 0)
			return;
		if (static_cast<size_t>(length) > text_capacity)
			length = static_cast<int>(text_capacity);
#if defined(__ANDROID__)
		__android_log_print(ANDROID_LOG_DEBUG, "Bungee", "%s", text);
#elif defined(__APPLE__)
		static const auto log = os_log_create("com.parabolaresearch.bungee", "diagnostics");
		os_log_info(log, "%{public}s", text);
#else
		memcpy(message, log_prefix, prefix_length);
		message[prefix_length + length] = '\n';
		log_write(message, prefix_length + length + 1);
#endif
	}
#endif
}

Instrumentation::Call::Call(Instrumentation &instrumentation, int sequence)
{
	if (sequence != instrumentation.expected)
	{
		static const char *names[] = {"specifyGrain", "analyseGrain", "synthesiseGrain"};
		instrumentation.log("FATAL: stretcher functions called in the wrong order: %s was called when expecting a call to %s", names[sequence], names[instrumentation.expected]);
		std::abort();
	}
	instrumentation.expected = (sequence + 1) % 3;

#ifdef EIGEN_RUNTIME_NO_MALLOC
	Eigen::internal::set_is_malloc_allowed(false);
#endif
}

Instrumentation::Call::~Call()
{
#ifdef EIGEN_RUNTIME_NO_MALLOC
	Eigen::internal::set_is_malloc_allowed(true);
#endif
}

} // namespace Bungee::Internal
