#pragma once

#include <cstddef>

#ifdef __cplusplus
extern "C"
{
#endif

	struct JsonString
	{
		/**
		 * The json string data. Is always null terminated.
		 */
		const char* JsonData;

		/**
		 * The length of the json string data. Does not include the null terminator.
		 */
		size_t StrSize;
	};

	struct ResourceMem
	{
		const void* ResourceData;
		size_t DataSize;
	};
	
	struct ResourceTypesArray
	{
		const char** Types;
		size_t TypeCount;
	};

	/**
	 * A read-only view of a string.
	 * The string is not guaranteed to be null terminated.
	 */
	struct StringView
	{
		const char* Data;
		size_t Size;
	};

#ifdef __cplusplus
}
#endif
